use bytes::Bytes;
use futures::StreamExt;
use serde::Serialize;
use std::future::Future;
use std::sync::Arc;
use tirea_agentos::contracts::{AgentEvent, ToolCallDecision};
use tirea_agentos::orchestrator::RunStream;
use tirea_agentos::runtime::loop_runner::RunCancellationToken;
use tirea_contract::ProtocolOutputEncoder;
use tokio::sync::{broadcast, mpsc};
use tracing::warn;

use super::http_sse::HttpSseServerEndpoint;
use super::{
    relay_binding, ChannelDownstreamEndpoint, RelayCancellation, SessionId, TranscoderEndpoint,
    TransportBinding, TransportCapabilities,
};

/// Wire an HTTP SSE relay for a single agent run.
///
/// Sets up the full pipeline: runtime endpoint → protocol transcoder →
/// transport binding → SSE server endpoint, then spawns the relay and
/// event pump tasks.
///
/// Returns the SSE byte receiver to feed into an HTTP response body.
///
/// `on_relay_done` is called after the relay finishes, receiving
/// the SSE sender (for trailer support).
pub fn wire_http_sse_relay<E, F, Fut>(
    run: RunStream,
    encoder: E,
    decision_ingress_rx: mpsc::UnboundedReceiver<ToolCallDecision>,
    cancellation_token: RunCancellationToken,
    fanout: Option<broadcast::Sender<Bytes>>,
    resumable_downstream: bool,
    protocol_label: &'static str,
    on_relay_done: F,
) -> mpsc::Receiver<Bytes>
where
    E: ProtocolOutputEncoder<InputEvent = AgentEvent> + Send + 'static,
    E::Event: Serialize + Send + 'static,
    F: FnOnce(mpsc::Sender<Bytes>) -> Fut + Send + 'static,
    Fut: Future<Output = ()> + Send,
{
    let thread_id = run.thread_id.clone();
    let (sse_tx, sse_rx) = mpsc::channel::<Bytes>(64);

    let upstream: Arc<HttpSseServerEndpoint<E::Event>> = match fanout {
        Some(f) => Arc::new(HttpSseServerEndpoint::with_fanout(
            decision_ingress_rx,
            sse_tx.clone(),
            f,
            cancellation_token,
        )),
        None => Arc::new(HttpSseServerEndpoint::new(
            decision_ingress_rx,
            sse_tx.clone(),
            cancellation_token,
        )),
    };

    let decision_tx = run.decision_tx.clone();
    let events = run.events;
    let (event_tx, event_rx) = mpsc::channel::<AgentEvent>(64);
    let runtime_ep = Arc::new(ChannelDownstreamEndpoint::new(event_rx, decision_tx));
    tokio::spawn(async move {
        let mut events = events;
        while let Some(e) = events.next().await {
            if event_tx.send(e).await.is_err() {
                break;
            }
        }
    });
    let downstream = Arc::new(TranscoderEndpoint::new(runtime_ep, encoder, Ok));

    let binding = TransportBinding {
        session: SessionId { thread_id },
        caps: TransportCapabilities {
            upstream_async: true,
            downstream_streaming: true,
            single_channel_bidirectional: false,
            resumable_downstream,
        },
        upstream,
        downstream,
    };
    let relay_cancel = RelayCancellation::new();
    tokio::spawn(async move {
        if let Err(err) = relay_binding(binding, relay_cancel.clone()).await {
            warn!(error = %err, "{protocol_label} transport relay failed");
        }
        on_relay_done(sse_tx).await;
    });

    sse_rx
}
