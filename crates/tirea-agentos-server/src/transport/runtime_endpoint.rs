//! RuntimeEndpoint: wraps a RunStream into Endpoint<AgentEvent, RuntimeInput>.

use async_trait::async_trait;
use futures::StreamExt;
use tirea_agentos::contracts::{AgentEvent, ToolCallDecision};
use tirea_agentos::orchestrator::RunStream;
use tirea_agentos::runtime::loop_runner::RunCancellationToken;
use tirea_contract::RuntimeInput;
use tokio::sync::{mpsc, Mutex};

use crate::transport::{BoxStream, Endpoint, TransportError};

const DEFAULT_EVENT_BUFFER: usize = 64;

/// Endpoint wrapping a [`RunStream`] into `Endpoint<AgentEvent, RuntimeInput>`.
///
/// - `recv()` returns the agent event stream (via an internal pump task).
/// - `send(RuntimeInput::Decision(d))` forwards to the run's decision channel.
/// - `send(RuntimeInput::Cancel)` triggers the cancellation token.
/// - `close()` is transport-only: does **not** trigger cancellation.
pub struct RuntimeEndpoint {
    event_rx: Mutex<Option<mpsc::Receiver<AgentEvent>>>,
    decision_tx: mpsc::UnboundedSender<ToolCallDecision>,
    cancellation_token: Option<RunCancellationToken>,
}

impl RuntimeEndpoint {
    /// Create from a [`RunStream`], spawning an internal event pump.
    ///
    /// `cancellation_token` is optional; when `None`, `Cancel` messages
    /// are silently ignored (suitable for transports without cancellation).
    pub fn new(run: RunStream, cancellation_token: Option<RunCancellationToken>) -> Self {
        Self::with_buffer(run, cancellation_token, DEFAULT_EVENT_BUFFER)
    }

    /// Create with an explicit event buffer size.
    pub fn with_buffer(
        run: RunStream,
        cancellation_token: Option<RunCancellationToken>,
        buffer: usize,
    ) -> Self {
        let decision_tx = run.decision_tx.clone();
        let events = run.events;
        let (event_tx, event_rx) = mpsc::channel::<AgentEvent>(buffer.max(1));

        tokio::spawn(async move {
            let mut events = events;
            while let Some(e) = events.next().await {
                if event_tx.send(e).await.is_err() {
                    break;
                }
            }
        });

        Self {
            event_rx: Mutex::new(Some(event_rx)),
            decision_tx,
            cancellation_token,
        }
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
            RuntimeInput::Decision(d) => {
                self.decision_tx.send(d).map_err(|_| TransportError::Closed)
            }
            RuntimeInput::Cancel => {
                if let Some(token) = &self.cancellation_token {
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

    #[tokio::test]
    async fn recv_delivers_agent_events() {
        let events = vec![
            AgentEvent::TextDelta {
                delta: "a".into(),
            },
            AgentEvent::TextDelta {
                delta: "b".into(),
            },
        ];
        let (run, _drx) = fake_run(events);
        let ep = RuntimeEndpoint::new(run, None);
        let stream = ep.recv().await.unwrap();
        let items: Vec<AgentEvent> = stream.map(|r| r.unwrap()).collect().await;
        assert_eq!(items.len(), 2);
    }

    #[tokio::test]
    async fn recv_called_twice_returns_closed() {
        let (run, _drx) = fake_run(vec![]);
        let ep = RuntimeEndpoint::new(run, None);
        let _first = ep.recv().await.unwrap();
        assert!(matches!(ep.recv().await, Err(TransportError::Closed)));
    }

    #[tokio::test]
    async fn send_decision_forwards_to_decision_tx() {
        let (run, mut drx) = fake_run(vec![]);
        let ep = RuntimeEndpoint::new(run, None);
        let d = ToolCallDecision::resume("tc1", serde_json::Value::Null, 0);
        ep.send(RuntimeInput::Decision(d.clone())).await.unwrap();
        let received = drx.recv().await.unwrap();
        assert_eq!(received, d);
    }

    #[tokio::test]
    async fn send_cancel_triggers_cancellation_token() {
        let (run, _drx) = fake_run(vec![]);
        let token = RunCancellationToken::new();
        let ep = RuntimeEndpoint::new(run, Some(token.clone()));
        assert!(!token.is_cancelled());
        ep.send(RuntimeInput::Cancel).await.unwrap();
        assert!(token.is_cancelled());
    }

    #[tokio::test]
    async fn send_cancel_without_token_is_noop() {
        let (run, _drx) = fake_run(vec![]);
        let ep = RuntimeEndpoint::new(run, None);
        ep.send(RuntimeInput::Cancel).await.unwrap();
    }

    #[tokio::test]
    async fn close_does_not_cancel() {
        let (run, _drx) = fake_run(vec![]);
        let token = RunCancellationToken::new();
        let ep = RuntimeEndpoint::new(run, Some(token.clone()));
        ep.close().await.unwrap();
        assert!(!token.is_cancelled(), "close must not cancel the run");
    }
}
