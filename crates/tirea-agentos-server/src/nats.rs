use crate::transport::nats::{NatsTransport, NatsTransportConfig};
use crate::transport::NatsProtocolError;

#[derive(Clone, Debug)]
pub struct NatsConfig {
    pub url: String,
    pub ag_ui_subject: String,
    pub ai_sdk_subject: String,
    pub outbound_buffer: usize,
}

impl NatsConfig {
    pub fn new(url: String) -> Self {
        Self {
            url,
            ag_ui_subject: "agentos.ag-ui.runs".to_string(),
            ai_sdk_subject: "agentos.ai-sdk.runs".to_string(),
            outbound_buffer: 64,
        }
    }

    pub async fn connect(&self) -> Result<NatsTransport, NatsProtocolError> {
        let client = async_nats::connect(&self.url).await?;
        Ok(NatsTransport::new(client, self.transport_config()))
    }

    fn transport_config(&self) -> NatsTransportConfig {
        NatsTransportConfig {
            outbound_buffer: self.outbound_buffer,
        }
    }
}
