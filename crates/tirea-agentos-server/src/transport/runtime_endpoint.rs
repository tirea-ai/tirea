//! RuntimeEndpoint: message-driven Endpoint<AgentEvent, RuntimeInput>.
//!
//! The endpoint lifecycle is fully driven by [`RuntimeInput`] messages:
//!
//! 1. `Run(request)` — starts execution via the injected run factory.
//! 2. `Decision(d)` — forwards to the running loop's decision channel.
//! 3. `Cancel` — triggers the cooperative cancellation token.
//!
//! `close()` is transport-only and does **not** cancel the run.

use std::future::Future;
use std::pin::Pin;

use async_trait::async_trait;
use futures::StreamExt;
use tirea_agentos::contracts::{AgentEvent, RunRequest, ToolCallDecision};
use tirea_agentos::orchestrator::RunStream;
use tirea_agentos::runtime::loop_runner::RunCancellationToken;
use tirea_contract::RuntimeInput;
use tokio::sync::{mpsc, Mutex};

use crate::transport::{BoxStream, Endpoint, TransportError};

const DEFAULT_EVENT_BUFFER: usize = 64;

/// Result produced by a run starter: the event stream + optional cancellation token.
type RunStartResult = Result<(RunStream, Option<RunCancellationToken>), TransportError>;

/// Async factory that prepares and executes a run from a [`RunRequest`].
///
/// Created by protocol handlers; captures `AgentOs`, resolved agent config,
/// and any protocol-specific state needed for run preparation.
pub type RunStarter =
    Box<dyn FnOnce(RunRequest) -> Pin<Box<dyn Future<Output = RunStartResult> + Send>> + Send>;

/// Message-driven runtime endpoint.
///
/// Implements `Endpoint<AgentEvent, RuntimeInput>`. The run is started
/// lazily when the first `RuntimeInput::Run` message arrives.
pub struct RuntimeEndpoint {
    event_tx: Mutex<Option<mpsc::Sender<AgentEvent>>>,
    event_rx: Mutex<Option<mpsc::Receiver<AgentEvent>>>,
    decision_tx: Mutex<Option<mpsc::UnboundedSender<ToolCallDecision>>>,
    cancellation_token: Mutex<Option<RunCancellationToken>>,
    run_starter: Mutex<Option<RunStarter>>,
}

impl RuntimeEndpoint {
    /// Create with a run factory that will be invoked on the first `Run` message.
    pub fn new(starter: RunStarter) -> Self {
        Self::with_buffer(starter, DEFAULT_EVENT_BUFFER)
    }

    /// Create with a run factory and explicit event buffer size.
    pub fn with_buffer(starter: RunStarter, buffer: usize) -> Self {
        let (event_tx, event_rx) = mpsc::channel::<AgentEvent>(buffer.max(1));
        Self {
            event_tx: Mutex::new(Some(event_tx)),
            event_rx: Mutex::new(Some(event_rx)),
            decision_tx: Mutex::new(None),
            cancellation_token: Mutex::new(None),
            run_starter: Mutex::new(Some(starter)),
        }
    }

    /// Attach an already-started run (bypasses the `Run` message).
    ///
    /// Useful for tests or contexts where the run was prepared externally.
    pub fn from_run_stream(
        run: RunStream,
        cancellation_token: Option<RunCancellationToken>,
    ) -> Self {
        Self::from_run_stream_with_buffer(run, cancellation_token, DEFAULT_EVENT_BUFFER)
    }

    /// Attach an already-started run with explicit buffer size.
    pub fn from_run_stream_with_buffer(
        run: RunStream,
        cancellation_token: Option<RunCancellationToken>,
        buffer: usize,
    ) -> Self {
        let (event_tx, event_rx) = mpsc::channel::<AgentEvent>(buffer.max(1));
        let decision_tx = run.decision_tx.clone();

        Self::spawn_event_pump(event_tx, run);

        Self {
            event_tx: Mutex::new(None),
            event_rx: Mutex::new(Some(event_rx)),
            decision_tx: Mutex::new(Some(decision_tx)),
            cancellation_token: Mutex::new(cancellation_token),
            run_starter: Mutex::new(None),
        }
    }

    /// Start the run from a `RunRequest` using the stored factory.
    async fn start_run(&self, request: RunRequest) -> Result<(), TransportError> {
        let starter = self
            .run_starter
            .lock()
            .await
            .take()
            .ok_or_else(|| TransportError::Internal("run already started".into()))?;

        let event_tx = self
            .event_tx
            .lock()
            .await
            .take()
            .ok_or_else(|| TransportError::Internal("event pump already started".into()))?;

        let (run, token) = starter(request).await?;

        *self.decision_tx.lock().await = Some(run.decision_tx.clone());
        *self.cancellation_token.lock().await = token;

        Self::spawn_event_pump(event_tx, run);

        Ok(())
    }

    fn spawn_event_pump(event_tx: mpsc::Sender<AgentEvent>, run: RunStream) {
        tokio::spawn(async move {
            let mut events = run.events;
            while let Some(e) = events.next().await {
                if event_tx.send(e).await.is_err() {
                    break;
                }
            }
            // event_tx is dropped here, closing the channel
        });
    }
}

#[async_trait]
impl Endpoint<AgentEvent, RuntimeInput> for RuntimeEndpoint {
    async fn recv(&self) -> Result<BoxStream<AgentEvent>, TransportError> {
        let mut guard = self.event_rx.lock().await;
        let mut rx = guard.take().ok_or(TransportError::Closed)?;
        let stream = async_stream::stream! {
            while let Some(item) = rx.recv().await {
                yield Ok(item);
            }
        };
        Ok(Box::pin(stream))
    }

    async fn send(&self, item: RuntimeInput) -> Result<(), TransportError> {
        match item {
            RuntimeInput::Run(request) => self.start_run(request).await,
            RuntimeInput::Decision(d) => {
                let guard = self.decision_tx.lock().await;
                guard
                    .as_ref()
                    .ok_or_else(|| TransportError::Internal("run not started".into()))?
                    .send(d)
                    .map_err(|_| TransportError::Closed)
            }
            RuntimeInput::Cancel => {
                let guard = self.cancellation_token.lock().await;
                if let Some(token) = guard.as_ref() {
                    token.cancel();
                }
                Ok(())
            }
        }
    }

    /// Transport-level close. Does **not** cancel the run.
    async fn close(&self) -> Result<(), TransportError> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::pin::Pin;
    use tirea_agentos::contracts::AgentEvent;

    fn test_run_request() -> RunRequest {
        RunRequest {
            agent_id: "test".into(),
            thread_id: None,
            run_id: None,
            parent_run_id: None,
            resource_id: None,
            state: None,
            messages: vec![],
            initial_decisions: vec![],
        }
    }

    fn fake_run(
        events: Vec<AgentEvent>,
    ) -> (RunStream, mpsc::UnboundedReceiver<ToolCallDecision>) {
        let (decision_tx, decision_rx) = mpsc::unbounded_channel();
        let (event_tx, event_rx) = mpsc::channel::<AgentEvent>(64);

        tokio::spawn(async move {
            for e in events {
                let _ = event_tx.send(e).await;
            }
        });

        let stream: Pin<Box<dyn futures::Stream<Item = AgentEvent> + Send>> =
            Box::pin(async_stream::stream! {
                let mut rx = event_rx;
                while let Some(item) = rx.recv().await {
                    yield item;
                }
            });

        let run = RunStream {
            thread_id: "t1".to_string(),
            run_id: "r1".to_string(),
            decision_tx,
            events: stream,
        };
        (run, decision_rx)
    }

    fn fake_starter(
        events: Vec<AgentEvent>,
    ) -> (RunStarter, mpsc::UnboundedReceiver<ToolCallDecision>) {
        let (run, drx) = fake_run(events);
        let starter: RunStarter = Box::new(move |_request| {
            Box::pin(async move { Ok((run, None)) })
        });
        (starter, drx)
    }

    // ── from_run_stream tests ───────────────────────────────────────

    #[tokio::test]
    async fn from_run_stream_recv_delivers_events() {
        let (run, _drx) = fake_run(vec![
            AgentEvent::TextDelta { delta: "a".into() },
            AgentEvent::TextDelta { delta: "b".into() },
        ]);
        let ep = RuntimeEndpoint::from_run_stream(run, None);
        let stream = ep.recv().await.unwrap();
        let items: Vec<AgentEvent> = stream.map(|r| r.unwrap()).collect().await;
        assert_eq!(items.len(), 2);
    }

    #[tokio::test]
    async fn from_run_stream_decision_forwarded() {
        let (run, mut drx) = fake_run(vec![]);
        let ep = RuntimeEndpoint::from_run_stream(run, None);
        let d = ToolCallDecision::resume("tc1", serde_json::Value::Null, 0);
        ep.send(RuntimeInput::Decision(d.clone())).await.unwrap();
        let received = drx.recv().await.unwrap();
        assert_eq!(received, d);
    }

    #[tokio::test]
    async fn from_run_stream_close_does_not_cancel() {
        let (run, _drx) = fake_run(vec![]);
        let token = RunCancellationToken::new();
        let ep = RuntimeEndpoint::from_run_stream(run, Some(token.clone()));
        ep.close().await.unwrap();
        assert!(!token.is_cancelled(), "close must not cancel the run");
    }

    // ── run starter tests ───────────────────────────────────────────

    #[tokio::test]
    async fn run_message_starts_execution() {
        let (starter, _drx) = fake_starter(vec![
            AgentEvent::TextDelta { delta: "x".into() },
        ]);
        let ep = RuntimeEndpoint::new(starter);
        let stream = ep.recv().await.unwrap();

        // Send Run to trigger the factory
        ep.send(RuntimeInput::Run(test_run_request())).await.unwrap();

        let items: Vec<AgentEvent> = stream.map(|r| r.unwrap()).collect().await;
        assert_eq!(items.len(), 1);
    }

    #[tokio::test]
    async fn decision_after_run_forwarded() {
        let (starter, mut drx) = fake_starter(vec![]);
        let ep = RuntimeEndpoint::new(starter);

        ep.send(RuntimeInput::Run(test_run_request())).await.unwrap();

        let d = ToolCallDecision::resume("tc1", serde_json::Value::Null, 0);
        ep.send(RuntimeInput::Decision(d.clone())).await.unwrap();
        let received = drx.recv().await.unwrap();
        assert_eq!(received, d);
    }

    #[tokio::test]
    async fn decision_before_run_returns_error() {
        let (starter, _drx) = fake_starter(vec![]);
        let ep = RuntimeEndpoint::new(starter);
        let d = ToolCallDecision::resume("tc1", serde_json::Value::Null, 0);
        let result = ep.send(RuntimeInput::Decision(d)).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn double_run_returns_error() {
        let (starter, _drx) = fake_starter(vec![]);
        let ep = RuntimeEndpoint::new(starter);
        ep.send(RuntimeInput::Run(test_run_request())).await.unwrap();
        let result = ep.send(RuntimeInput::Run(test_run_request())).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn cancel_triggers_token() {
        let token = RunCancellationToken::new();
        let token_clone = token.clone();
        let (run, _drx) = fake_run(vec![]);
        let starter: RunStarter = Box::new(move |_request| {
            Box::pin(async move { Ok((run, Some(token_clone))) })
        });
        let ep = RuntimeEndpoint::new(starter);

        ep.send(RuntimeInput::Run(test_run_request())).await.unwrap();
        assert!(!token.is_cancelled());

        ep.send(RuntimeInput::Cancel).await.unwrap();
        assert!(token.is_cancelled());
    }

    #[tokio::test]
    async fn recv_called_twice_returns_closed() {
        let (starter, _drx) = fake_starter(vec![]);
        let ep = RuntimeEndpoint::new(starter);
        let _first = ep.recv().await.unwrap();
        assert!(matches!(ep.recv().await, Err(TransportError::Closed)));
    }
}
