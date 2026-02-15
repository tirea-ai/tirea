use carve_agent::orchestrator::{AgentOs, RunExtensions, RunRequest};
use carve_agent::runtime::streaming::AgentEvent;
use carve_protocol_ag_ui::{AGUIEvent, AgUiInputAdapter, AgUiProtocolEncoder, RunAgentRequest};
use carve_protocol_ag_ui_runtime::build_agui_extensions;
use carve_protocol_ai_sdk_v6::{
    AiSdkV6InputAdapter, AiSdkV6ProtocolEncoder, AiSdkV6RunRequest, UIStreamEvent,
};
use carve_protocol_contract::{ProtocolInputAdapter, ProtocolOutputEncoder};
use futures::StreamExt;
use serde::Deserialize;
use serde::Serialize;
use std::sync::Arc;
use tracing;

use crate::transport::pump_encoded_stream;
use async_nats::ConnectErrorKind;

const SUBJECT_RUN_AGUI: &str = "agentos.run.agui";
const SUBJECT_RUN_AISDK: &str = "agentos.run.aisdk";

#[derive(Debug, thiserror::Error)]
pub enum NatsGatewayError {
    #[error("nats connect error: {0}")]
    Connect(#[from] async_nats::error::Error<ConnectErrorKind>),

    #[error("nats subscribe error: {0}")]
    Subscribe(#[from] async_nats::SubscribeError),

    #[error("nats error: {0}")]
    Nats(#[from] async_nats::Error),

    #[error("bad request: {0}")]
    BadRequest(String),

    #[error("run error: {0}")]
    Run(String),
}

#[derive(Clone)]
pub struct NatsGateway {
    os: Arc<AgentOs>,
    client: async_nats::Client,
}

impl NatsGateway {
    pub async fn connect(os: Arc<AgentOs>, nats_url: &str) -> Result<Self, NatsGatewayError> {
        let client = async_nats::connect(nats_url).await?;
        Ok(Self { os, client })
    }

    pub async fn serve(self) -> Result<(), NatsGatewayError> {
        let mut sub_agui = self.client.subscribe(SUBJECT_RUN_AGUI).await?;
        let mut sub_aisdk = self.client.subscribe(SUBJECT_RUN_AISDK).await?;

        let this = Arc::new(self);
        loop {
            tokio::select! {
                Some(msg) = sub_agui.next() => {
                    let this = this.clone();
                    tokio::spawn(async move {
                        if let Err(e) = this.handle_agui(msg).await {
                            tracing::error!(error = %e, "nats agui handler failed");
                        }
                    });
                }
                Some(msg) = sub_aisdk.next() => {
                    let this = this.clone();
                    tokio::spawn(async move {
                        if let Err(e) = this.handle_aisdk(msg).await {
                            tracing::error!(error = %e, "nats aisdk handler failed");
                        }
                    });
                }
                else => break,
            }
        }
        Ok(())
    }

    async fn handle_agui(
        self: Arc<Self>,
        msg: async_nats::Message,
    ) -> Result<(), NatsGatewayError> {
        #[derive(Debug, Deserialize)]
        struct Req {
            #[serde(rename = "agentId")]
            agent_id: String,
            request: RunAgentRequest,
            #[serde(rename = "replySubject")]
            reply_subject: Option<String>,
        }

        let req: Req = serde_json::from_slice(&msg.payload)
            .map_err(|e| NatsGatewayError::BadRequest(e.to_string()))?;
        req.request
            .validate()
            .map_err(|e| NatsGatewayError::BadRequest(e.to_string()))?;

        let reply = msg.reply.or(req.reply_subject.map(Into::into));
        let Some(reply) = reply else {
            return Err(NatsGatewayError::BadRequest(
                "missing reply subject".to_string(),
            ));
        };

        let extensions = build_agui_extensions(&req.request);
        let run_request = AgUiInputAdapter::to_run_request(req.agent_id, req.request);
        self.run_and_publish(
            run_request,
            extensions,
            reply,
            |run| AgUiProtocolEncoder::new(run.thread_id.clone(), run.run_id.clone()),
            |msg| AGUIEvent::run_error(msg, None),
        )
        .await
    }

    async fn handle_aisdk(
        self: Arc<Self>,
        msg: async_nats::Message,
    ) -> Result<(), NatsGatewayError> {
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
            .map_err(|e| NatsGatewayError::BadRequest(e.to_string()))?;
        if req.input.trim().is_empty() {
            return Err(NatsGatewayError::BadRequest(
                "input cannot be empty".to_string(),
            ));
        }

        let reply = msg.reply.or(req.reply_subject.map(Into::into));
        let Some(reply) = reply else {
            return Err(NatsGatewayError::BadRequest(
                "missing reply subject".to_string(),
            ));
        };

        let run_request = AiSdkV6InputAdapter::to_run_request(
            req.agent_id,
            AiSdkV6RunRequest {
                thread_id: req.thread_id,
                input: req.input,
                run_id: req.run_id,
            },
        );

        self.run_and_publish(
            run_request,
            RunExtensions::default(),
            reply,
            |run| AiSdkV6ProtocolEncoder::new(run.run_id.clone(), Some(run.thread_id.clone())),
            UIStreamEvent::error,
        )
        .await
    }

    async fn run_and_publish<E, ErrEvent, BuildEncoder, BuildErrorEvent>(
        &self,
        run_request: RunRequest,
        extensions: RunExtensions,
        reply: async_nats::Subject,
        build_encoder: BuildEncoder,
        build_error_event: BuildErrorEvent,
    ) -> Result<(), NatsGatewayError>
    where
        E: ProtocolOutputEncoder<InputEvent = AgentEvent> + Send + 'static,
        E::Event: Serialize + Send + 'static,
        ErrEvent: Serialize,
        BuildEncoder: FnOnce(&carve_agent::orchestrator::RunStream) -> E,
        BuildErrorEvent: FnOnce(String) -> ErrEvent,
    {
        let run = match self
            .os
            .run_stream_with_extensions(run_request, extensions)
            .await
        {
            Ok(run) => run,
            Err(err) => {
                let event = build_error_event(err.to_string());
                let payload = serde_json::to_vec(&event)
                    .map_err(|e| {
                        NatsGatewayError::Run(format!("serialize error event failed: {e}"))
                    })?
                    .into();
                if let Err(publish_err) = self.client.publish(reply, payload).await {
                    return Err(NatsGatewayError::Run(format!(
                        "publish error event failed: {publish_err}"
                    )));
                }
                return Ok(());
            }
        };

        let encoder = build_encoder(&run);
        let client = self.client.clone();
        pump_encoded_stream(run.events, encoder, move |event| {
            let client = client.clone();
            let reply = reply.clone();
            async move {
                let payload = match serde_json::to_vec(&event) {
                    Ok(payload) => payload.into(),
                    Err(err) => {
                        tracing::warn!(error = %err, "failed to serialize NATS protocol event");
                        return Err(());
                    }
                };
                client.publish(reply, payload).await.map_err(|_| ())
            }
        })
        .await;

        Ok(())
    }
}
