use serde::Deserialize;
use std::sync::Arc;
use tirea_agentos::orchestrator::AgentOs;
use tirea_contract::ProtocolInputAdapter;
use tirea_protocol_ag_ui::{
    apply_agui_extensions, AgUiInputAdapter, AgUiProtocolEncoder, Event, RunAgentInput,
};

use crate::nats::NatsConfig;
use crate::transport::nats::{run_and_publish, serve_nats, NatsTransportConfig};
use crate::transport::NatsProtocolError;

/// Serve AG-UI protocol over NATS using config.
pub async fn serve(
    client: async_nats::Client,
    os: Arc<AgentOs>,
    config: &NatsConfig,
) -> Result<(), NatsProtocolError> {
    serve_nats(
        client,
        &config.ag_ui_subject,
        config.transport_config(),
        "agui",
        move |client, msg, transport_config| {
            let os = os.clone();
            async move { handle_message(client, os, msg, transport_config).await }
        },
    )
    .await
}

async fn handle_message(
    client: async_nats::Client,
    os: Arc<AgentOs>,
    msg: async_nats::Message,
    transport_config: NatsTransportConfig,
) -> Result<(), NatsProtocolError> {
    #[derive(Debug, Deserialize)]
    struct Req {
        #[serde(rename = "agentId")]
        agent_id: String,
        request: RunAgentInput,
        #[serde(rename = "replySubject")]
        reply_subject: Option<String>,
    }

    let req: Req = serde_json::from_slice(&msg.payload)
        .map_err(|e| NatsProtocolError::BadRequest(e.to_string()))?;
    req.request
        .validate()
        .map_err(|e| NatsProtocolError::BadRequest(e.to_string()))?;

    let reply = msg.reply.or(req.reply_subject.map(Into::into));
    let Some(reply) = reply else {
        return Err(NatsProtocolError::BadRequest(
            "missing reply subject".to_string(),
        ));
    };

    let resolved = match os.resolve(&req.agent_id) {
        Ok(r) => r,
        Err(err) => {
            let event = Event::run_error(err.to_string(), None);
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

    let mut resolved = resolved;
    apply_agui_extensions(&mut resolved, &req.request);
    let run_request = AgUiInputAdapter::to_run_request(req.agent_id, req.request);

    run_and_publish(
        os.as_ref(),
        run_request,
        resolved,
        reply,
        client,
        transport_config,
        |run| AgUiProtocolEncoder::new(run.thread_id.clone(), run.run_id.clone()),
        |msg| Event::run_error(msg, None),
    )
    .await
}
