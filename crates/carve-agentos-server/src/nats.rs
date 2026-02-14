use carve_agent::ag_ui::{AGUIEvent, RunAgentRequest};
use carve_agent::ui_stream::UIStreamEvent;
use carve_agent::AgentOs;
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tracing;

use crate::protocol::{
    AgUiInputAdapter, AgUiProtocolEncoder, AiSdkInputAdapter, AiSdkProtocolEncoder, AiSdkRunRequest,
    ProtocolInputAdapter, ProtocolOutputEncoder,
};
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

        let run_request = AgUiInputAdapter::to_run_request(req.agent_id, req.request);

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

        let enc = AgUiProtocolEncoder::new(run.thread_id, run.run_id);
        publish_encoded_nats_stream(self.client.clone(), reply, run.events, enc).await;

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

        let run_request = AiSdkInputAdapter::to_run_request(
            req.agent_id,
            AiSdkRunRequest {
                thread_id: req.thread_id,
                input: req.input,
                run_id: req.run_id,
            },
        );

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

        let enc = AiSdkProtocolEncoder::new(run.run_id, Some(run.thread_id));
        publish_encoded_nats_stream(self.client.clone(), reply, run.events, enc).await;

        Ok(())
    }
}

async fn publish_nats_json<T: Serialize>(
    client: &async_nats::Client,
    subject: &async_nats::Subject,
    event: &T,
) -> Result<(), ()> {
    let payload = serde_json::to_vec(event).unwrap_or_default().into();
    client
        .publish(subject.clone(), payload)
        .await
        .map_err(|_| ())
}

async fn publish_encoded_nats_stream<E>(
    client: async_nats::Client,
    subject: async_nats::Subject,
    mut events: std::pin::Pin<Box<dyn futures::Stream<Item = carve_agent::AgentEvent> + Send>>,
    mut encoder: E,
) where
    E: ProtocolOutputEncoder,
{
    for event in encoder.prologue() {
        if publish_nats_json(&client, &subject, &event).await.is_err() {
            return;
        }
    }

    while let Some(agent_event) = events.next().await {
        for event in encoder.on_agent_event(&agent_event) {
            if publish_nats_json(&client, &subject, &event).await.is_err() {
                return;
            }
        }
    }

    for event in encoder.epilogue() {
        if publish_nats_json(&client, &subject, &event).await.is_err() {
            return;
        }
    }
}
