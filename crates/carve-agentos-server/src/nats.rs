use carve_agent::ag_ui::{AGUIEvent, MessageRole, RunAgentRequest};
use carve_agent::ui_stream::UIStreamEvent;
use carve_agent::{AgentOs, Message, Role, RunRequest, Visibility};
use futures::StreamExt;
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::Arc;
use tracing;

use crate::protocol::{AgUiEncoder, AiSdkEncoder};
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
    pub async fn connect(
        os: Arc<AgentOs>,
        nats_url: &str,
    ) -> Result<Self, NatsGatewayError> {
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

        // Convert AG-UI protocol → internal RunRequest
        let mut runtime = HashMap::new();
        if let Some(ref parent_run_id) = req.request.parent_run_id {
            runtime.insert(
                "parent_run_id".to_string(),
                serde_json::Value::String(parent_run_id.clone()),
            );
        }

        let messages = convert_agui_messages(&req.request.messages);

        let run_request = RunRequest {
            agent_id: req.agent_id,
            thread_id: Some(req.request.thread_id.clone()),
            run_id: Some(req.request.run_id.clone()),
            resource_id: None,
            initial_state: req.request.state.clone(),
            messages,
            runtime,
        };

        let run = match self.os.run_stream(run_request).await {
            Ok(r) => r,
            Err(e) => {
                let err = AGUIEvent::run_error(e.to_string(), None);
                let _ = self
                    .client
                    .publish(reply, serde_json::to_vec(&err).unwrap_or_default().into())
                    .await;
                return Ok(());
            }
        };

        let mut events = run.events;
        let mut enc = AgUiEncoder::new(run.thread_id.clone(), run.run_id.clone());

        while let Some(ev) = events.next().await {
            for ag in enc.on_agent_event(&ev) {
                let _ = self
                    .client
                    .publish(
                        reply.clone(),
                        serde_json::to_vec(&ag).unwrap_or_default().into(),
                    )
                    .await;
            }
        }

        for fallback in enc.fallback_finished(&run.thread_id, &run.run_id) {
            let _ = self
                .client
                .publish(
                    reply.clone(),
                    serde_json::to_vec(&fallback).unwrap_or_default().into(),
                )
                .await;
        }

        Ok(())
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

        let run_request = RunRequest {
            agent_id: req.agent_id,
            thread_id: if req.thread_id.trim().is_empty() {
                None
            } else {
                Some(req.thread_id)
            },
            run_id: req.run_id,
            resource_id: None,
            initial_state: None,
            messages: vec![Message::user(req.input)],
            runtime: HashMap::new(),
        };

        let run = match self.os.run_stream(run_request).await {
            Ok(r) => r,
            Err(e) => {
                let err = UIStreamEvent::error(e.to_string());
                let _ = self
                    .client
                    .publish(reply, serde_json::to_vec(&err).unwrap_or_default().into())
                    .await;
                return Ok(());
            }
        };

        let mut events = run.events;
        let mut enc = AiSdkEncoder::new(run.run_id.clone());

        for e in enc.prologue() {
            let _ = self
                .client
                .publish(
                    reply.clone(),
                    serde_json::to_vec(&e).unwrap_or_default().into(),
                )
                .await;
        }

        while let Some(ev) = events.next().await {
            for ui in enc.on_agent_event(&ev) {
                let _ = self
                    .client
                    .publish(
                        reply.clone(),
                        serde_json::to_vec(&ui).unwrap_or_default().into(),
                    )
                    .await;
            }
        }

        Ok(())
    }
}

/// Convert AG-UI messages to internal format, filtering out assistant messages.
fn convert_agui_messages(messages: &[carve_agent::ag_ui::AGUIMessage]) -> Vec<Message> {
    messages
        .iter()
        .filter(|m| m.role != MessageRole::Assistant)
        .map(|m| {
            let role = match m.role {
                MessageRole::System | MessageRole::Developer => Role::System,
                MessageRole::User => Role::User,
                MessageRole::Assistant => Role::Assistant,
                MessageRole::Tool => Role::Tool,
            };
            Message {
                id: m.id.clone(),
                role,
                content: m.content.clone(),
                tool_calls: None,
                tool_call_id: m.tool_call_id.clone(),
                visibility: Visibility::default(),
                metadata: None,
            }
        })
        .collect()
}
