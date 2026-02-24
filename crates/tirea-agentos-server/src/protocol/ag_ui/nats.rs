use futures::StreamExt;
use serde::Deserialize;
use std::sync::Arc;
use tirea_agentos::orchestrator::AgentOs;
use tirea_contract::ProtocolInputAdapter;
use tirea_protocol_ag_ui::{
    apply_agui_extensions, AgUiInputAdapter, AgUiProtocolEncoder, Event, RunAgentInput,
};

use crate::protocol::nats_runtime::run_and_publish;
use crate::protocol::NatsProtocolError;

/// Default AG-UI run subject.
pub const RUN_SUBJECT: &str = "agentos.ag-ui.runs";

/// Serve AG-UI protocol over NATS using the default subject.
pub async fn serve(client: async_nats::Client, os: Arc<AgentOs>) -> Result<(), NatsProtocolError> {
    serve_on_subject(client, os, RUN_SUBJECT).await
}

/// Serve AG-UI protocol over NATS on a custom subject.
pub async fn serve_on_subject(
    client: async_nats::Client,
    os: Arc<AgentOs>,
    subject: &str,
) -> Result<(), NatsProtocolError> {
    let mut sub = client.subscribe(subject.to_string()).await?;
    while let Some(msg) = sub.next().await {
        let client = client.clone();
        let os = os.clone();
        tokio::spawn(async move {
            if let Err(e) = handle_message(client, os, msg).await {
                tracing::error!(error = %e, "nats agui handler failed");
            }
        });
    }
    Ok(())
}

async fn handle_message(
    client: async_nats::Client,
    os: Arc<AgentOs>,
    msg: async_nats::Message,
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
        |run| AgUiProtocolEncoder::new(run.thread_id.clone(), run.run_id.clone()),
        |msg| Event::run_error(msg, None),
    )
    .await
}
