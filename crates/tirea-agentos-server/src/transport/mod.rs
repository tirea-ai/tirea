pub mod http_run;
pub mod http_sse;
pub mod nats;
mod nats_error;
pub mod runtime_endpoint;
pub mod transcoder;

pub use nats_error::NatsProtocolError;
pub use runtime_endpoint::{RunStarter, RuntimeEndpoint};
pub use transcoder::TranscoderEndpoint;

use async_trait::async_trait;
use futures::Stream;
use futures::StreamExt;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
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
    /// Upstream (caller→runtime) supports asynchronous delivery.
    pub upstream_async: bool,
    /// Downstream (runtime→caller) delivers events as a stream.
    pub downstream_streaming: bool,
    /// Both directions share a single bidirectional channel.
    pub single_channel_bidirectional: bool,
    /// Downstream stream can be resumed after reconnection.
    pub resumable_downstream: bool,
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

/// Generic downstream endpoint backed by a bounded `mpsc::Receiver` for recv
/// and an unbounded `mpsc::UnboundedSender` for send.
///
/// Test-only: used in integration tests to simulate a runtime endpoint.
#[doc(hidden)]
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

/// A matched pair of endpoints representing both sides of a transport channel.
///
/// - `server`: the runtime/handler side — receives `Ingress`, sends `Egress`
/// - `client`: the caller/consumer side — receives `Egress`, sends `Ingress`
pub struct EndpointPair<Ingress, Egress>
where
    Ingress: Send + 'static,
    Egress: Send + 'static,
{
    pub server: Arc<dyn Endpoint<Ingress, Egress>>,
    pub client: Arc<dyn Endpoint<Egress, Ingress>>,
}

/// Create an in-memory `EndpointPair` backed by bounded channels.
pub fn channel_pair<A, B>(buffer: usize) -> EndpointPair<A, B>
where
    A: Send + 'static,
    B: Send + 'static,
{
    let buffer = buffer.max(1);
    let (a_tx, a_rx) = mpsc::channel::<A>(buffer);
    let (b_tx, b_rx) = mpsc::channel::<B>(buffer);

    let server = Arc::new(BoundedChannelEndpoint::new(a_rx, b_tx));
    let client = Arc::new(BoundedChannelEndpoint::new(b_rx, a_tx));

    EndpointPair { server, client }
}

/// Channel endpoint backed by bounded `mpsc` channels. Used internally by [`channel_pair`].
struct BoundedChannelEndpoint<RecvMsg, SendMsg>
where
    RecvMsg: Send + 'static,
    SendMsg: Send + 'static,
{
    recv_rx: Mutex<Option<mpsc::Receiver<RecvMsg>>>,
    send_tx: mpsc::Sender<SendMsg>,
}

impl<RecvMsg, SendMsg> BoundedChannelEndpoint<RecvMsg, SendMsg>
where
    RecvMsg: Send + 'static,
    SendMsg: Send + 'static,
{
    fn new(recv_rx: mpsc::Receiver<RecvMsg>, send_tx: mpsc::Sender<SendMsg>) -> Self {
        Self {
            recv_rx: Mutex::new(Some(recv_rx)),
            send_tx,
        }
    }
}

#[async_trait]
impl<RecvMsg, SendMsg> Endpoint<RecvMsg, SendMsg> for BoundedChannelEndpoint<RecvMsg, SendMsg>
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
        self.send_tx
            .send(item)
            .await
            .map_err(|_| TransportError::Closed)
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

    // ── ChannelDownstreamEndpoint ──────────────────────────────────

    #[tokio::test]
    async fn channel_downstream_recv_called_twice_returns_closed() {
        let (_tx, rx) = mpsc::channel::<u32>(4);
        let (send_tx, _send_rx) = mpsc::unbounded_channel::<String>();
        let ep = ChannelDownstreamEndpoint::new(rx, send_tx);

        let _first = ep.recv().await.unwrap();
        let second = ep.recv().await;
        assert!(matches!(second, Err(TransportError::Closed)));
    }

    #[tokio::test]
    async fn channel_downstream_send_after_receiver_dropped_returns_closed() {
        let (_tx, rx) = mpsc::channel::<u32>(4);
        let (send_tx, send_rx) = mpsc::unbounded_channel::<String>();
        let ep = ChannelDownstreamEndpoint::new(rx, send_tx);

        drop(send_rx);
        let result = ep.send("msg".to_string()).await;
        assert!(matches!(result, Err(TransportError::Closed)));
    }

    #[tokio::test]
    async fn channel_downstream_recv_delivers_all_items_in_order() {
        let (tx, rx) = mpsc::channel::<u32>(8);
        let (send_tx, _send_rx) = mpsc::unbounded_channel::<String>();
        let ep = ChannelDownstreamEndpoint::new(rx, send_tx);

        for i in 0..5 {
            tx.send(i).await.unwrap();
        }
        drop(tx);

        let stream = ep.recv().await.unwrap();
        let items: Vec<u32> = stream.map(|r| r.unwrap()).collect().await;
        assert_eq!(items, vec![0, 1, 2, 3, 4]);
    }

    // ── channel_pair / BoundedChannelEndpoint ───────────────────

    #[tokio::test]
    async fn channel_pair_bidirectional() {
        let pair = channel_pair::<u32, String>(4);

        // server sends String, client receives String
        pair.server.send("hello".to_string()).await.unwrap();
        let mut client_stream = pair.client.recv().await.unwrap();
        let received = client_stream.next().await.unwrap().unwrap();
        assert_eq!(received, "hello");

        // client sends u32, server receives u32
        pair.client.send(42).await.unwrap();
        let mut server_stream = pair.server.recv().await.unwrap();
        let received = server_stream.next().await.unwrap().unwrap();
        assert_eq!(received, 42);
    }

    #[tokio::test]
    async fn channel_pair_close_propagates() {
        let pair = channel_pair::<u32, String>(4);

        pair.server.send("a".to_string()).await.unwrap();
        drop(pair.server);

        let mut stream = pair.client.recv().await.unwrap();
        let first = stream.next().await.unwrap().unwrap();
        assert_eq!(first, "a");

        // After server side dropped, stream ends
        assert!(stream.next().await.is_none());
    }

    #[tokio::test]
    async fn channel_pair_recv_called_twice_returns_closed() {
        let pair = channel_pair::<u32, String>(4);

        let _first = pair.server.recv().await.unwrap();
        let second = pair.server.recv().await;
        assert!(matches!(second, Err(TransportError::Closed)));
    }

    #[tokio::test]
    async fn channel_pair_send_after_peer_dropped_returns_closed() {
        let pair = channel_pair::<u32, String>(4);

        drop(pair.client);
        // server sends String → client's recv channel, but client is dropped
        let result = pair.server.send("orphan".to_string()).await;
        assert!(matches!(result, Err(TransportError::Closed)));
    }

    #[tokio::test]
    async fn channel_pair_multiple_items_preserve_order() {
        let pair = channel_pair::<u32, String>(8);

        for i in 0..5 {
            pair.client.send(i).await.unwrap();
        }
        drop(pair.client);

        let stream = pair.server.recv().await.unwrap();
        let items: Vec<u32> = stream.map(|r| r.unwrap()).collect().await;
        assert_eq!(items, vec![0, 1, 2, 3, 4]);
    }

    #[tokio::test]
    async fn channel_pair_concurrent_bidirectional() {
        let pair = channel_pair::<u32, String>(8);

        // Start consumers that read exactly 3 items each.
        // (Cannot use .collect() because each endpoint's send_tx keeps
        // the peer's recv channel open — a known trait of paired endpoints.)
        let consumer_server = tokio::spawn({
            let server = pair.server.clone();
            async move {
                let mut stream = server.recv().await.unwrap();
                let mut items = Vec::new();
                for _ in 0..3 {
                    items.push(stream.next().await.unwrap().unwrap());
                }
                items
            }
        });
        let consumer_client = tokio::spawn({
            let client = pair.client.clone();
            async move {
                let mut stream = client.recv().await.unwrap();
                let mut items = Vec::new();
                for _ in 0..3 {
                    items.push(stream.next().await.unwrap().unwrap());
                }
                items
            }
        });

        // Concurrently send in both directions
        for i in 0u32..3 {
            pair.client.send(i).await.unwrap();
        }
        for s in ["a", "b", "c"] {
            pair.server.send(s.to_string()).await.unwrap();
        }

        let server_items = consumer_server.await.unwrap();
        assert_eq!(server_items, vec![0, 1, 2]);

        let client_items = consumer_client.await.unwrap();
        assert_eq!(client_items, vec!["a", "b", "c"]);
    }

    // ── relay_binding edge cases ────────────────────────────────

    #[tokio::test]
    async fn relay_completes_when_downstream_closes() {
        let (up_in_tx, up_in_rx) = mpsc::unbounded_channel::<u32>();
        let (up_send_tx, _up_send_rx) = mpsc::unbounded_channel::<String>();

        let (_down_in_tx, down_in_rx) = mpsc::unbounded_channel::<String>();
        let (down_send_tx, mut down_send_rx) = mpsc::unbounded_channel::<u32>();

        let upstream = Arc::new(ChannelEndpoint::new(up_in_rx, up_send_tx));
        let downstream = Arc::new(ChannelEndpoint::new(down_in_rx, down_send_tx));

        let binding = TransportBinding {
            session: SessionId {
                thread_id: "t".to_string(),
            },
            caps: TransportCapabilities::default(),
            upstream,
            downstream,
        };

        let cancel = RelayCancellation::new();
        let relay = tokio::spawn(relay_binding(binding, cancel));

        // Send one message upstream → downstream, then close upstream ingress
        up_in_tx.send(42).unwrap();
        drop(up_in_tx);
        // Also close downstream egress source so relay's egress loop ends
        drop(_down_in_tx);

        let received = down_send_rx.recv().await.unwrap();
        assert_eq!(received, 42);

        let result = relay.await.unwrap();
        assert!(result.is_ok(), "relay should normalize Closed to Ok");
    }

    #[tokio::test]
    async fn relay_with_cancel_before_messages() {
        let (_up_tx, up_rx) = mpsc::unbounded_channel::<u32>();
        let (up_send_tx, _up_send_rx) = mpsc::unbounded_channel::<String>();
        let (_down_tx, down_rx) = mpsc::unbounded_channel::<String>();
        let (down_send_tx, _down_send_rx) = mpsc::unbounded_channel::<u32>();

        let upstream = Arc::new(ChannelEndpoint::new(up_rx, up_send_tx));
        let downstream = Arc::new(ChannelEndpoint::new(down_rx, down_send_tx));

        let binding = TransportBinding {
            session: SessionId {
                thread_id: "t".to_string(),
            },
            caps: TransportCapabilities::default(),
            upstream,
            downstream,
        };

        let cancel = RelayCancellation::new();
        cancel.cancel();
        // Close sources so streams end
        drop(_up_tx);
        drop(_down_tx);

        let result = relay_binding(binding, cancel).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn relay_multiple_messages_in_sequence() {
        let (up_tx, up_rx) = mpsc::unbounded_channel::<u32>();
        let (up_send_tx, mut up_send_rx) = mpsc::unbounded_channel::<String>();
        let (down_tx, down_rx) = mpsc::unbounded_channel::<String>();
        let (down_send_tx, mut down_send_rx) = mpsc::unbounded_channel::<u32>();

        let upstream = Arc::new(ChannelEndpoint::new(up_rx, up_send_tx));
        let downstream = Arc::new(ChannelEndpoint::new(down_rx, down_send_tx));

        let binding = TransportBinding {
            session: SessionId {
                thread_id: "seq".to_string(),
            },
            caps: TransportCapabilities::default(),
            upstream,
            downstream,
        };

        let cancel = RelayCancellation::new();
        let relay = tokio::spawn(relay_binding(binding, cancel));

        // ingress: upstream → downstream
        for i in 0..3 {
            up_tx.send(i).unwrap();
        }
        for expected in 0..3 {
            assert_eq!(down_send_rx.recv().await.unwrap(), expected);
        }

        // egress: downstream → upstream
        for s in ["x", "y", "z"] {
            down_tx.send(s.to_string()).unwrap();
        }
        for expected in ["x", "y", "z"] {
            assert_eq!(up_send_rx.recv().await.unwrap(), expected);
        }

        drop(up_tx);
        drop(down_tx);
        assert!(relay.await.unwrap().is_ok());
    }
}
