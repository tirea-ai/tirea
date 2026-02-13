use carve_agent::ag_ui::AGUIEvent;
use carve_agent::ui_stream::UIStreamEvent;
use carve_agent::{
    apply_agui_request_to_session, AgentOs, Message, RunAgentRequest, RunContext, Session, Storage,
    AGUI_REQUEST_APPLIED_RUNTIME_KEY,
};
use futures::StreamExt;
use serde::Deserialize;
use std::sync::Arc;
use tracing;

use crate::ids::generate_run_id;
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
        Ok(Self {
            os,
            storage,
            client,
        })
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
        let mut session = apply_agui_request_to_session(session, &req.request);
        if session.messages.len() != before_messages
            || session.patches.len() != before_patches
            || session.state != before_state
        {
            self.storage
                .save(&session)
                .await
                .map_err(|e| NatsGatewayError::BadRequest(e.to_string()))?;
        }
        let _ = session
            .runtime
            .set(AGUI_REQUEST_APPLIED_RUNTIME_KEY, req.request.run_id.clone());

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
                    if let Err(e) = storage.save(&cp.session).await {
                        tracing::error!(session_id = %cp.session.id, error = %e, "failed to save checkpoint");
                    }
                }
                if let Ok(final_session) = final_session.await {
                    if let Err(e) = storage.save(&final_session).await {
                        tracing::error!(session_id = %final_session.id, error = %e, "failed to save final session");
                    }
                }
            });
        }

        while let Some(ev) = inner.next().await {
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

        for fallback in enc.fallback_finished(&req.request.thread_id, &req.request.run_id) {
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

        // Validate agent exists before mutating session state.
        if let Err(e) = self.os.validate_agent(&req.agent_id) {
            let err = UIStreamEvent::error(e.to_string());
            let _ = self
                .client
                .publish(
                    reply.clone(),
                    serde_json::to_vec(&err).unwrap_or_default().into(),
                )
                .await;
            return Ok(());
        }

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

        // Set run_id on the session runtime if provided; otherwise it will be auto-generated by the loop
        let run_id = if let Some(run_id) = req.run_id.clone() {
            let _ = session.runtime.set("run_id", run_id.clone());
            run_id
        } else {
            // Generate a run_id for the encoder, but don't set it on runtime - let the loop auto-generate
            generate_run_id()
        };

        let run_ctx = RunContext {
            cancellation_token: None,
        };

        let stream_with_checkpoints =
            match self
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

        // Wait for the first event to extract the actual run_id from RunStart
        let mut events = stream_with_checkpoints.events;
        let first_event = events.next().await;
        let actual_run_id =
            if let Some(carve_agent::AgentEvent::RunStart { run_id: id, .. }) = &first_event {
                id.clone()
            } else {
                run_id.clone() // Fallback to the provided/generated run_id
            };

        let mut enc = AiSdkEncoder::new(actual_run_id.clone());
        for e in enc.prologue() {
            let _ = self
                .client
                .publish(
                    reply.clone(),
                    serde_json::to_vec(&e).unwrap_or_default().into(),
                )
                .await;
        }

        {
            let mut checkpoints = stream_with_checkpoints.checkpoints;
            let final_session = stream_with_checkpoints.final_session;
            let storage = self.storage.clone();
            tokio::spawn(async move {
                while let Some(cp) = checkpoints.recv().await {
                    if let Err(e) = storage.save(&cp.session).await {
                        tracing::error!(session_id = %cp.session.id, error = %e, "failed to save checkpoint");
                    }
                }
                if let Ok(final_session) = final_session.await {
                    if let Err(e) = storage.save(&final_session).await {
                        tracing::error!(session_id = %final_session.id, error = %e, "failed to save final session");
                    }
                }
            });
        }

        // Process the first event if we got one
        if let Some(ev) = first_event {
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
