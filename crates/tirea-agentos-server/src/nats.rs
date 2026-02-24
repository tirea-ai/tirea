use std::sync::Arc;
use tirea_agentos::orchestrator::AgentOs;

use crate::protocol::{self, NatsProtocolError};

pub use crate::protocol::ag_ui::nats::RUN_SUBJECT as SUBJECT_RUN_AGUI;
pub use crate::protocol::ai_sdk_v6::nats::RUN_SUBJECT as SUBJECT_RUN_AISDK;

pub type NatsGatewayError = NatsProtocolError;

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

    /// Deprecated combined NATS gateway.
    ///
    /// Prefer explicit protocol composition:
    /// - `protocol::ag_ui::nats::serve(...)`
    /// - `protocol::ai_sdk_v6::nats::serve(...)`
    #[deprecated(
        since = "0.1.1",
        note = "compose protocol::ag_ui::nats::serve and protocol::ai_sdk_v6::nats::serve explicitly"
    )]
    pub async fn serve(self) -> Result<(), NatsGatewayError> {
        let agui_client = self.client.clone();
        let aisdk_client = self.client.clone();
        let os_for_agui = self.os.clone();
        let os_for_aisdk = self.os.clone();

        let agui_task =
            tokio::spawn(
                async move { protocol::ag_ui::nats::serve(agui_client, os_for_agui).await },
            );
        let aisdk_task = tokio::spawn(async move {
            protocol::ai_sdk_v6::nats::serve(aisdk_client, os_for_aisdk).await
        });

        tokio::select! {
            res = agui_task => {
                match res {
                    Ok(inner) => inner,
                    Err(err) => Err(NatsProtocolError::Run(format!("agui serve task join error: {err}"))),
                }
            }
            res = aisdk_task => {
                match res {
                    Ok(inner) => inner,
                    Err(err) => Err(NatsProtocolError::Run(format!("ai-sdk serve task join error: {err}"))),
                }
            }
        }
    }
}
