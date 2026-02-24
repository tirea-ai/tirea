use async_trait::async_trait;
use futures::Stream;
use futures::StreamExt;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tirea_agentos::contracts::AgentEvent;
use tirea_contract::ProtocolOutputEncoder;
use tokio::sync::{mpsc, Mutex};

/// Common boxed stream for transport endpoints.
pub type BoxStream<T> = Pin<Box<dyn Stream<Item = Result<T, TransportError>> + Send>>;

/// Session key for one chat transport binding.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct SessionId {
    pub thread_id: String,
}

/// Transport-level capability declaration used for composition checks.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TransportCapabilities {
    pub upstream_async: bool,
    pub downstream_streaming: bool,
    pub single_channel_bidirectional: bool,
    pub resumable_downstream: bool,
}

/// Required transport capabilities requested by a protocol/router path.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RequiredCapabilities {
    pub upstream_async: bool,
    pub downstream_streaming: bool,
    pub require_single_channel_bidirectional: bool,
}

/// Returns true when the transport capabilities satisfy requirements.
pub fn capabilities_match(req: RequiredCapabilities, got: TransportCapabilities) -> bool {
    (!req.upstream_async || got.upstream_async)
        && (!req.downstream_streaming || got.downstream_streaming)
        && (!req.require_single_channel_bidirectional || got.single_channel_bidirectional)
}

#[derive(Debug, thiserror::Error)]
pub enum TransportError {
    #[error("session not found: {0}")]
    SessionNotFound(String),
    #[error("closed")]
    Closed,
    #[error("io: {0}")]
    Io(String),
    #[error("internal: {0}")]
    Internal(String),
}

/// Lightweight cancellation token for relay loops.
#[derive(Clone, Default, Debug)]
pub struct RelayCancellation {
    cancelled: Arc<AtomicBool>,
}

impl RelayCancellation {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Relaxed);
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Relaxed)
    }
}

/// Generic endpoint view.
///
/// A caller only needs recv/send from one side;
/// direction is encoded at type-level by `RecvMsg` and `SendMsg`.
#[async_trait]
pub trait Endpoint<RecvMsg, SendMsg>: Send + Sync
where
    RecvMsg: Send + 'static,
    SendMsg: Send + 'static,
{
    async fn recv(&self) -> Result<BoxStream<RecvMsg>, TransportError>;
    async fn send(&self, item: SendMsg) -> Result<(), TransportError>;
    async fn close(&self) -> Result<(), TransportError>;
}

/// Generic downstream endpoint backed by one receiver + one sender.
pub struct ChannelDownstreamEndpoint<RecvMsg, SendMsg>
where
    RecvMsg: Send + 'static,
    SendMsg: Send + 'static,
{
    recv_rx: Mutex<Option<mpsc::Receiver<RecvMsg>>>,
    send_tx: mpsc::UnboundedSender<SendMsg>,
}

impl<RecvMsg, SendMsg> ChannelDownstreamEndpoint<RecvMsg, SendMsg>
where
    RecvMsg: Send + 'static,
    SendMsg: Send + 'static,
{
    pub fn new(recv_rx: mpsc::Receiver<RecvMsg>, send_tx: mpsc::UnboundedSender<SendMsg>) -> Self {
        Self {
            recv_rx: Mutex::new(Some(recv_rx)),
            send_tx,
        }
    }
}

#[async_trait]
impl<RecvMsg, SendMsg> Endpoint<RecvMsg, SendMsg> for ChannelDownstreamEndpoint<RecvMsg, SendMsg>
where
    RecvMsg: Send + 'static,
    SendMsg: Send + 'static,
{
    async fn recv(&self) -> Result<BoxStream<RecvMsg>, TransportError> {
        let mut guard = self.recv_rx.lock().await;
        let mut rx = guard.take().ok_or(TransportError::Closed)?;
        let stream = async_stream::stream! {
            while let Some(item) = rx.recv().await {
                yield Ok(item);
            }
        };
        Ok(Box::pin(stream))
    }

    async fn send(&self, item: SendMsg) -> Result<(), TransportError> {
        self.send_tx.send(item).map_err(|_| TransportError::Closed)
    }

    async fn close(&self) -> Result<(), TransportError> {
        Ok(())
    }
}

/// Bound transport session with both sides.
///
/// - `upstream`: caller-facing side: recv `UpMsg`, send `DownMsg`
/// - `downstream`: runtime/next-hop side: recv `DownMsg`, send `UpMsg`
pub struct TransportBinding<UpMsg, DownMsg>
where
    UpMsg: Send + 'static,
    DownMsg: Send + 'static,
{
    pub session: SessionId,
    pub caps: TransportCapabilities,
    pub upstream: Arc<dyn Endpoint<UpMsg, DownMsg>>,
    pub downstream: Arc<dyn Endpoint<DownMsg, UpMsg>>,
}

/// Factory that binds one transport session.
#[async_trait]
pub trait TransportFactory<UpMsg, DownMsg>: Send + Sync
where
    UpMsg: Send + 'static,
    DownMsg: Send + 'static,
{
    async fn bind_session(
        &self,
        session: SessionId,
    ) -> Result<TransportBinding<UpMsg, DownMsg>, TransportError>;
}

/// Relay one bound session bidirectionally:
/// - upstream.recv -> downstream.send
/// - downstream.recv -> upstream.send
pub async fn relay_binding<UpMsg, DownMsg>(
    binding: TransportBinding<UpMsg, DownMsg>,
    cancel: RelayCancellation,
) -> Result<(), TransportError>
where
    UpMsg: Send + 'static,
    DownMsg: Send + 'static,
{
    let upstream = binding.upstream.clone();
    let downstream = binding.downstream.clone();

    let ingress = {
        let cancel = cancel.clone();
        let upstream = upstream.clone();
        let downstream = downstream.clone();
        tokio::spawn(async move {
            let mut stream = upstream.recv().await?;
            while let Some(item) = stream.next().await {
                if cancel.is_cancelled() {
                    break;
                }
                downstream.send(item?).await?;
            }
            Ok::<(), TransportError>(())
        })
    };

    let egress = {
        let cancel = cancel.clone();
        let upstream = upstream.clone();
        let downstream = downstream.clone();
        tokio::spawn(async move {
            let mut stream = downstream.recv().await?;
            while let Some(item) = stream.next().await {
                if cancel.is_cancelled() {
                    break;
                }
                upstream.send(item?).await?;
            }
            Ok::<(), TransportError>(())
        })
    };

    fn normalize_relay_result(result: Result<(), TransportError>) -> Result<(), TransportError> {
        match result {
            Ok(()) | Err(TransportError::Closed) => Ok(()),
            Err(other) => Err(other),
        }
    }

    let egress_res = egress
        .await
        .map_err(|e| TransportError::Internal(e.to_string()))?;
    cancel.cancel();
    ingress.abort();
    normalize_relay_result(egress_res)
}

/// Pump an internal agent event stream through a protocol output encoder and
/// forward each encoded event to the provided async sink callback.
pub async fn pump_encoded_stream<E, SendFn, SendFut>(
    mut events: Pin<Box<dyn Stream<Item = AgentEvent> + Send>>,
    mut encoder: E,
    mut send: SendFn,
) where
    E: ProtocolOutputEncoder<InputEvent = AgentEvent>,
    SendFn: FnMut(E::Event) -> SendFut,
    SendFut: Future<Output = Result<(), ()>>,
{
    for event in encoder.prologue() {
        if send(event).await.is_err() {
            return;
        }
    }

    while let Some(agent_event) = events.next().await {
        for event in encoder.on_agent_event(&agent_event) {
            if send(event).await.is_err() {
                return;
            }
        }
    }

    for event in encoder.epilogue() {
        if send(event).await.is_err() {
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use tokio::sync::mpsc;

    #[derive(Debug)]
    struct ChannelEndpoint<Recv, SendMsg>
    where
        Recv: std::marker::Send + 'static,
        SendMsg: std::marker::Send + 'static,
    {
        recv_rx: tokio::sync::Mutex<Option<mpsc::UnboundedReceiver<Recv>>>,
        send_tx: mpsc::UnboundedSender<SendMsg>,
    }

    impl<Recv, SendMsg> ChannelEndpoint<Recv, SendMsg>
    where
        Recv: std::marker::Send + 'static,
        SendMsg: std::marker::Send + 'static,
    {
        fn new(
            recv_rx: mpsc::UnboundedReceiver<Recv>,
            send_tx: mpsc::UnboundedSender<SendMsg>,
        ) -> Self {
            Self {
                recv_rx: tokio::sync::Mutex::new(Some(recv_rx)),
                send_tx,
            }
        }
    }

    #[async_trait]
    impl<Recv, SendMsg> Endpoint<Recv, SendMsg> for ChannelEndpoint<Recv, SendMsg>
    where
        Recv: std::marker::Send + 'static,
        SendMsg: std::marker::Send + 'static,
    {
        async fn recv(&self) -> Result<BoxStream<Recv>, TransportError> {
            let mut guard = self.recv_rx.lock().await;
            let rx = guard.take().ok_or(TransportError::Closed)?;
            let stream = async_stream::stream! {
                let mut rx = rx;
                while let Some(item) = rx.recv().await {
                    yield Ok(item);
                }
            };
            Ok(Box::pin(stream))
        }

        async fn send(&self, item: SendMsg) -> Result<(), TransportError> {
            self.send_tx.send(item).map_err(|_| TransportError::Closed)
        }

        async fn close(&self) -> Result<(), TransportError> {
            Ok(())
        }
    }

    #[test]
    fn capabilities_match_respects_required_single_channel() {
        let req = RequiredCapabilities {
            upstream_async: true,
            downstream_streaming: true,
            require_single_channel_bidirectional: true,
        };
        let got = TransportCapabilities {
            upstream_async: true,
            downstream_streaming: true,
            single_channel_bidirectional: false,
            resumable_downstream: true,
        };
        assert!(!capabilities_match(req, got));

        let got = TransportCapabilities {
            single_channel_bidirectional: true,
            ..got
        };
        assert!(capabilities_match(req, got));
    }

    #[tokio::test]
    async fn relay_binding_moves_messages_both_directions() {
        let (up_in_tx, up_in_rx) = mpsc::unbounded_channel::<u32>();
        let (up_send_tx, mut up_send_rx) = mpsc::unbounded_channel::<String>();

        let (down_in_tx, down_in_rx) = mpsc::unbounded_channel::<String>();
        let (down_send_tx, mut down_send_rx) = mpsc::unbounded_channel::<u32>();

        let upstream = Arc::new(ChannelEndpoint::new(up_in_rx, up_send_tx));
        let downstream = Arc::new(ChannelEndpoint::new(down_in_rx, down_send_tx));

        let binding = TransportBinding {
            session: SessionId {
                thread_id: "thread-1".to_string(),
            },
            caps: TransportCapabilities {
                upstream_async: true,
                downstream_streaming: true,
                single_channel_bidirectional: false,
                resumable_downstream: true,
            },
            upstream,
            downstream,
        };

        let cancel = RelayCancellation::new();
        let relay_task = tokio::spawn(relay_binding(binding, cancel.clone()));

        up_in_tx.send(7).unwrap();
        down_in_tx.send("evt".to_string()).unwrap();

        let up_out = up_send_rx
            .recv()
            .await
            .expect("upstream should receive event");
        let down_out = down_send_rx
            .recv()
            .await
            .expect("downstream should receive ingress");

        assert_eq!(up_out, "evt");
        assert_eq!(down_out, 7);

        cancel.cancel();
        drop(up_in_tx);
        drop(down_in_tx);

        let result = relay_task.await.expect("relay task should join");
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn channel_downstream_endpoint_bridges_recv_and_send() {
        let (recv_tx, recv_rx) = mpsc::channel::<u32>(4);
        let (send_tx, mut send_rx) = mpsc::unbounded_channel::<String>();
        let endpoint = ChannelDownstreamEndpoint::new(recv_rx, send_tx);

        recv_tx.send(7).await.expect("seed recv channel");
        drop(recv_tx);

        let mut stream = endpoint.recv().await.expect("recv stream");
        let first = stream
            .next()
            .await
            .expect("stream item")
            .expect("stream ok item");
        assert_eq!(first, 7);

        endpoint
            .send("ok".to_string())
            .await
            .expect("send should work");
        let sent = send_rx.recv().await.expect("sent item");
        assert_eq!(sent, "ok");
    }
}
