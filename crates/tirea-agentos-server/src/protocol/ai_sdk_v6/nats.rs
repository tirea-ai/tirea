use futures::StreamExt;
use serde::Deserialize;
use std::sync::Arc;
use tirea_agentos::orchestrator::AgentOs;
use tirea_contract::ProtocolInputAdapter;
use tirea_protocol_ai_sdk_v6::{
    apply_ai_sdk_extensions, AiSdkV6InputAdapter, AiSdkV6ProtocolEncoder, AiSdkV6RunRequest,
    UIStreamEvent,
};

use crate::protocol::nats_runtime::{run_and_publish, NatsTransportConfig};
use crate::protocol::NatsProtocolError;

/// Default AI SDK v6 run subject.
pub const RUN_SUBJECT: &str = "agentos.ai-sdk.runs";

/// Serve AI SDK v6 protocol over NATS using the default subject.
pub async fn serve(client: async_nats::Client, os: Arc<AgentOs>) -> Result<(), NatsProtocolError> {
    serve_on_subject(client, os, RUN_SUBJECT).await
}

/// Serve AI SDK v6 protocol over NATS on a custom subject.
pub async fn serve_on_subject(
    client: async_nats::Client,
    os: Arc<AgentOs>,
    subject: &str,
) -> Result<(), NatsProtocolError> {
    serve_on_subject_with_transport_config(client, os, subject, NatsTransportConfig::default())
        .await
}

pub async fn serve_on_subject_with_transport_config(
    client: async_nats::Client,
    os: Arc<AgentOs>,
    subject: &str,
    transport_config: NatsTransportConfig,
) -> Result<(), NatsProtocolError> {
    let mut sub = client.subscribe(subject.to_string()).await?;
    while let Some(msg) = sub.next().await {
        let client = client.clone();
        let os = os.clone();
        let transport_config = transport_config.clone();
        tokio::spawn(async move {
            if let Err(e) = handle_message(client, os, msg, transport_config).await {
                tracing::error!(error = %e, "nats aisdk handler failed");
            }
        });
    }
    Ok(())
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
        #[serde(rename = "sessionId")]
        thread_id: String,
        input: String,
        #[serde(rename = "runId")]
        run_id: Option<String>,
        #[serde(rename = "replySubject")]
        reply_subject: Option<String>,
    }

    let req: Req = serde_json::from_slice(&msg.payload)
        .map_err(|e| NatsProtocolError::BadRequest(e.to_string()))?;
    if req.input.trim().is_empty() {
        return Err(NatsProtocolError::BadRequest(
            "input cannot be empty".to_string(),
        ));
    }

    let reply = msg.reply.or(req.reply_subject.map(Into::into));
    let Some(reply) = reply else {
        return Err(NatsProtocolError::BadRequest(
            "missing reply subject".to_string(),
        ));
    };

    let mut resolved = match os.resolve(&req.agent_id) {
        Ok(r) => r,
        Err(err) => {
            let event = UIStreamEvent::error(err.to_string());
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

    let req_for_runtime =
        AiSdkV6RunRequest::from_thread_input(req.thread_id, req.input, req.run_id);
    apply_ai_sdk_extensions(&mut resolved, &req_for_runtime);
    let run_request = AiSdkV6InputAdapter::to_run_request(req.agent_id, req_for_runtime);

    run_and_publish(
        os.as_ref(),
        run_request,
        resolved,
        reply,
        client,
        transport_config,
        |run| AiSdkV6ProtocolEncoder::new(run.run_id.clone(), Some(run.thread_id.clone())),
        UIStreamEvent::error,
    )
    .await
}
