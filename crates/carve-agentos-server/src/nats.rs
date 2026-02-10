use carve_agent::ag_ui::AGUIEvent;
use carve_agent::ui_stream::UIStreamEvent;
use carve_agent::{
    apply_agui_request_to_session, AgentOs, Message, RunAgentRequest, RunContext, Session, Storage,
};
use futures::StreamExt;
use serde::Deserialize;
use std::sync::Arc;
use uuid::Uuid;

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
}

#[derive(Clone)]
pub struct NatsGateway {
    os: Arc<AgentOs>,
    storage: Arc<dyn Storage>,
    client: async_nats::Client,
}

impl NatsGateway {
    pub async fn connect(
        os: Arc<AgentOs>,
        storage: Arc<dyn Storage>,
        nats_url: &str,
    ) -> Result<Self, NatsGatewayError> {
        let client = async_nats::connect(nats_url).await?;
        Ok(Self { os, storage, client })
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
                        let _ = this.handle_agui(msg).await;
                    });
                }
                Some(msg) = sub_aisdk.next() => {
                    let this = this.clone();
                    tokio::spawn(async move {
                        let _ = this.handle_aisdk(msg).await;
                    });
                }
                else => break,
            }
        }
        Ok(())
    }

    async fn handle_agui(self: Arc<Self>, msg: async_nats::Message) -> Result<(), NatsGatewayError> {
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

        let session = self
            .storage
            .load(&req.request.thread_id)
            .await
            .map_err(|e| NatsGatewayError::BadRequest(e.to_string()))?
            .unwrap_or_else(|| {
                if let Some(state) = req.request.state.clone() {
                    Session::with_initial_state(req.request.thread_id.clone(), state)
                } else {
                    Session::new(req.request.thread_id.clone())
                }
            });
        if session.id != req.request.thread_id {
            return Err(NatsGatewayError::BadRequest(
                "stored session id does not match threadId".to_string(),
            ));
        }

        // Industry-common: persist inbound messages/tool responses before running.
        let before_messages = session.messages.len();
        let before_patches = session.patches.len();
        let before_state = session.state.clone();
        let session = apply_agui_request_to_session(session, &req.request);
        if session.messages.len() != before_messages
            || session.patches.len() != before_patches
            || session.state != before_state
        {
            self.storage
                .save(&session)
                .await
                .map_err(|e| NatsGatewayError::BadRequest(e.to_string()))?;
        }

        let (client, cfg, tools, session) = match self.os.resolve(&req.agent_id, session) {
            Ok(w) => w,
            Err(e) => {
                let err = AGUIEvent::run_error(e.to_string(), None);
                let _ = self
                    .client
                    .publish(reply, serde_json::to_vec(&err).unwrap_or_default().into())
                    .await;
                return Ok(());
            }
        };

        let stream_with_checkpoints = carve_agent::run_agent_events_with_request_checkpoints(
            client,
            cfg,
            session,
            tools,
            req.request.clone(),
        );

        let mut inner = stream_with_checkpoints.events;
        let mut enc = AgUiEncoder::new(req.request.thread_id.clone(), req.request.run_id.clone());

        {
            let mut checkpoints = stream_with_checkpoints.checkpoints;
            let final_session = stream_with_checkpoints.final_session;
            let storage = self.storage.clone();
            tokio::spawn(async move {
                while let Some(cp) = checkpoints.recv().await {
                    let _ = storage.save(&cp.session).await;
                }
                if let Ok(final_session) = final_session.await {
                    let _ = storage.save(&final_session).await;
                }
            });
        }

        while let Some(ev) = inner.next().await {
            for ag in enc.on_agent_event(&ev) {
                let _ = self
                    .client
                    .publish(reply.clone(), serde_json::to_vec(&ag).unwrap_or_default().into())
                    .await;
            }
        }

        if let Some(fallback) = enc.fallback_finished(&req.request.thread_id, &req.request.run_id) {
            let _ = self
                .client
                .publish(reply.clone(), serde_json::to_vec(&fallback).unwrap_or_default().into())
                .await;
        }

        Ok(())
    }

    async fn handle_aisdk(self: Arc<Self>, msg: async_nats::Message) -> Result<(), NatsGatewayError> {
        #[derive(Debug, Deserialize)]
        struct Req {
            #[serde(rename = "agentId")]
            agent_id: String,
            #[serde(rename = "sessionId")]
            session_id: String,
            input: String,
            #[serde(rename = "runId")]
            run_id: Option<String>,
            #[serde(rename = "replySubject")]
            reply_subject: Option<String>,
        }

        let req: Req = serde_json::from_slice(&msg.payload)
            .map_err(|e| NatsGatewayError::BadRequest(e.to_string()))?;
        if req.session_id.trim().is_empty() || req.input.trim().is_empty() {
            return Err(NatsGatewayError::BadRequest(
                "sessionId/input cannot be empty".to_string(),
            ));
        }

        let reply = msg.reply.or(req.reply_subject.map(Into::into));
        let Some(reply) = reply else {
            return Err(NatsGatewayError::BadRequest(
                "missing reply subject".to_string(),
            ));
        };

        let mut session = self
            .storage
            .load(&req.session_id)
            .await
            .map_err(|e| NatsGatewayError::BadRequest(e.to_string()))?
            .unwrap_or_else(|| Session::new(req.session_id.clone()));
        session = session.with_message(Message::user(req.input));

        // Industry-common: persist the user message immediately.
        self.storage
            .save(&session)
            .await
            .map_err(|e| NatsGatewayError::BadRequest(e.to_string()))?;

        let run_id = req
            .run_id
            .unwrap_or_else(|| Uuid::new_v4().simple().to_string());
        let run_ctx = RunContext {
            run_id: Some(run_id.clone()),
            parent_run_id: None,
        };

        let stream_with_checkpoints = match self
            .os
            .run_stream_with_checkpoints(&req.agent_id, session, run_ctx)
        {
            Ok(s) => s,
            Err(e) => {
                let err = UIStreamEvent::error(e.to_string());
                let _ = self
                    .client
                    .publish(reply, serde_json::to_vec(&err).unwrap_or_default().into())
                    .await;
                return Ok(());
            }
        };

        let mut enc = AiSdkEncoder::new(run_id.clone());
        for e in enc.prologue() {
            let _ = self
                .client
                .publish(reply.clone(), serde_json::to_vec(&e).unwrap_or_default().into())
                .await;
        }

        {
            let mut checkpoints = stream_with_checkpoints.checkpoints;
            let final_session = stream_with_checkpoints.final_session;
            let storage = self.storage.clone();
            tokio::spawn(async move {
                while let Some(cp) = checkpoints.recv().await {
                    let _ = storage.save(&cp.session).await;
                }
                if let Ok(final_session) = final_session.await {
                    let _ = storage.save(&final_session).await;
                }
            });
        }

        let mut events = stream_with_checkpoints.events;
        while let Some(ev) = events.next().await {
            for ui in enc.on_agent_event(&ev) {
                let _ = self
                    .client
                    .publish(reply.clone(), serde_json::to_vec(&ui).unwrap_or_default().into())
                    .await;
            }
        }

        Ok(())
    }
}
