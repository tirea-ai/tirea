use futures::StreamExt;
use serde::Serialize;
use std::future::Future;
use std::sync::Arc;
use tirea_agentos::contracts::{AgentEvent, RunRequest, ToolCallDecision};
use tirea_agentos::orchestrator::{AgentOs, ResolvedRun, RunStream};
use tirea_contract::ProtocolOutputEncoder;
use tokio::sync::mpsc;

use crate::transport::NatsProtocolError;
use crate::transport::{
    pump_encoded_stream, relay_binding, ChannelDownstreamEndpoint, Endpoint, RelayCancellation,
    SessionId, TransportBinding, TransportCapabilities, TransportError,
};

#[derive(Clone, Debug)]
pub struct NatsTransportConfig {
    pub outbound_buffer: usize,
}

impl Default for NatsTransportConfig {
    fn default() -> Self {
        Self {
            outbound_buffer: 64,
        }
    }
}

/// Subscribe to a NATS subject and dispatch each message to a handler.
///
/// Shared subscribe-loop used by both AG-UI and AI SDK NATS protocol servers.
pub async fn serve_nats<H, Fut>(
    client: async_nats::Client,
    subject: &str,
    transport_config: NatsTransportConfig,
    protocol_label: &'static str,
    handler: H,
) -> Result<(), NatsProtocolError>
where
    H: Fn(async_nats::Client, async_nats::Message, NatsTransportConfig) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Result<(), NatsProtocolError>> + Send + 'static,
{
    let handler = Arc::new(handler);
    let mut sub = client.subscribe(subject.to_string()).await?;
    while let Some(msg) = sub.next().await {
        let client = client.clone();
        let transport_config = transport_config.clone();
        let handler = handler.clone();
        tokio::spawn(async move {
            if let Err(e) = handler(client, msg, transport_config).await {
                tracing::error!(error = %e, "nats {protocol_label} handler failed");
            }
        });
    }
    Ok(())
}

pub async fn run_and_publish<E, ErrEvent, BuildEncoder, BuildErrorEvent>(
    os: &AgentOs,
    run_request: RunRequest,
    resolved: ResolvedRun,
    reply: async_nats::Subject,
    client: async_nats::Client,
    transport_config: NatsTransportConfig,
    build_encoder: BuildEncoder,
    build_error_event: BuildErrorEvent,
) -> Result<(), NatsProtocolError>
where
    E: ProtocolOutputEncoder<InputEvent = AgentEvent> + Send + 'static,
    E::Event: Serialize + Send + 'static,
    ErrEvent: Serialize,
    BuildEncoder: FnOnce(&RunStream) -> E,
    BuildErrorEvent: FnOnce(String) -> ErrEvent,
{
    let run = match os.prepare_run(run_request, resolved).await {
        Ok(prepared) => match AgentOs::execute_prepared(prepared) {
            Ok(run) => run,
            Err(err) => {
                let event = build_error_event(err.to_string());
                let payload = serde_json::to_vec(&event)
                    .map_err(|e| {
                        NatsProtocolError::Run(format!("serialize error event failed: {e}"))
                    })?
                    .into();
                if let Err(publish_err) = client.publish(reply, payload).await {
                    return Err(NatsProtocolError::Run(format!(
                        "publish error event failed: {publish_err}"
                    )));
                }
                return Ok(());
            }
        },
        Err(err) => {
            let event = build_error_event(err.to_string());
            let payload = serde_json::to_vec(&event)
                .map_err(|e| NatsProtocolError::Run(format!("serialize error event failed: {e}")))?
                .into();
            if let Err(publish_err) = client.publish(reply, payload).await {
                return Err(NatsProtocolError::Run(format!(
                    "publish error event failed: {publish_err}"
                )));
            }
            return Ok(());
        }
    };

    let session_thread_id = run.thread_id.clone();
    let encoder = build_encoder(&run);
    let upstream = Arc::new(NatsReplyUpstreamEndpoint::new(client, reply));
    let decision_tx = run.decision_tx.clone();
    let (tx, rx) = mpsc::channel::<E::Event>(transport_config.outbound_buffer.max(1));
    tokio::spawn(async move {
        let tx_events = tx.clone();
        pump_encoded_stream(run.events, encoder, move |event| {
            let tx = tx_events.clone();
            async move { tx.send(event).await.map_err(|_| ()) }
        })
        .await;
    });
    let downstream = Arc::new(ChannelDownstreamEndpoint::new(rx, decision_tx));
    let binding = TransportBinding {
        session: SessionId {
            thread_id: session_thread_id,
        },
        caps: TransportCapabilities {
            upstream_async: false,
            downstream_streaming: true,
            single_channel_bidirectional: false,
            resumable_downstream: false,
        },
        upstream,
        downstream,
    };
    relay_binding(binding, RelayCancellation::new())
        .await
        .map_err(|e| NatsProtocolError::Run(format!("transport relay failed: {e}")))?;

    Ok(())
}

struct NatsReplyUpstreamEndpoint {
    client: async_nats::Client,
    reply: async_nats::Subject,
}

impl NatsReplyUpstreamEndpoint {
    fn new(client: async_nats::Client, reply: async_nats::Subject) -> Self {
        Self { client, reply }
    }
}

#[async_trait::async_trait]
impl<Evt> Endpoint<ToolCallDecision, Evt> for NatsReplyUpstreamEndpoint
where
    Evt: Serialize + Send + 'static,
{
    async fn recv(&self) -> Result<crate::transport::BoxStream<ToolCallDecision>, TransportError> {
        let stream = futures::stream::empty::<Result<ToolCallDecision, TransportError>>();
        Ok(Box::pin(stream))
    }

    async fn send(&self, item: Evt) -> Result<(), TransportError> {
        let payload = serde_json::to_vec(&item).map_err(|e| {
            tracing::warn!(error = %e, "failed to serialize NATS protocol event");
            TransportError::Io(format!("serialize event failed: {e}"))
        })?;
        self.client
            .publish(self.reply.clone(), payload.into())
            .await
            .map_err(|e| TransportError::Io(e.to_string()))
    }

    async fn close(&self) -> Result<(), TransportError> {
        Ok(())
    }
}
