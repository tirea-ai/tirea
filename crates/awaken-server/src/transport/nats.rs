//! NATS transport integration (feature-gated behind "nats").
//!
//! Provides [`NatsTransport`] for serving agent events over NATS publish/subscribe.
//! Each request arrives on a subscribed subject and events are published to the
//! reply subject using a protocol encoder (AG-UI or AI SDK).

use std::future::Future;
use std::sync::Arc;

use awaken_contract::contract::event::AgentEvent;
use awaken_contract::contract::transport::Transcoder;
use serde::Serialize;

/// NATS transport configuration.
#[derive(Clone, Debug)]
pub struct NatsTransportConfig {
    /// Maximum number of outbound events buffered before back-pressure.
    pub outbound_buffer: usize,
}

impl Default for NatsTransportConfig {
    fn default() -> Self {
        Self {
            outbound_buffer: 64,
        }
    }
}

/// Error type for NATS protocol operations.
#[derive(Debug, thiserror::Error)]
pub enum NatsProtocolError {
    #[error("nats connect error: {0}")]
    Connect(#[from] async_nats::error::Error<async_nats::ConnectErrorKind>),

    #[error("nats subscribe error: {0}")]
    Subscribe(#[from] async_nats::SubscribeError),

    #[error("nats error: {0}")]
    Nats(#[from] async_nats::Error),

    #[error("bad request: {0}")]
    BadRequest(String),

    #[error("run error: {0}")]
    Run(String),
}

/// Owns a NATS connection and transport configuration.
#[derive(Clone)]
pub struct NatsTransport {
    client: async_nats::Client,
    config: NatsTransportConfig,
}

impl NatsTransport {
    /// Create a new transport with an existing NATS client connection.
    pub fn new(client: async_nats::Client, config: NatsTransportConfig) -> Self {
        Self { client, config }
    }

    /// Access the underlying NATS client.
    pub fn client(&self) -> &async_nats::Client {
        &self.client
    }

    /// Access the transport configuration.
    pub fn config(&self) -> &NatsTransportConfig {
        &self.config
    }

    /// Subscribe to a NATS subject and dispatch each message to a handler.
    ///
    /// Spawns a tokio task per incoming message. The `protocol_label` is used in
    /// error log messages to identify which protocol handler failed.
    pub async fn serve<H, Fut>(
        &self,
        subject: &str,
        protocol_label: &'static str,
        handler: H,
    ) -> Result<(), NatsProtocolError>
    where
        H: Fn(NatsTransport, async_nats::Message) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<(), NatsProtocolError>> + Send + 'static,
    {
        use futures::StreamExt;
        let handler = Arc::new(handler);
        let mut sub = self.client.subscribe(subject.to_string()).await?;
        while let Some(msg) = sub.next().await {
            let transport = self.clone();
            let handler = handler.clone();
            tokio::spawn(async move {
                if let Err(e) = handler(transport, msg).await {
                    tracing::error!(error = %e, "nats {protocol_label} handler failed");
                }
            });
        }
        Ok(())
    }

    /// Publish a sequence of agent events to a NATS reply subject.
    ///
    /// Consumes events from the given channel, encodes each through the provided
    /// encoder, and publishes the serialized output to `reply`.
    pub async fn publish_events<E>(
        &self,
        mut rx: tokio::sync::mpsc::Receiver<AgentEvent>,
        reply: async_nats::Subject,
        mut encoder: E,
    ) -> Result<(), NatsProtocolError>
    where
        E: Transcoder<Input = AgentEvent>,
        E::Output: Serialize + Send + 'static,
    {
        // Emit prologue
        for item in encoder.prologue() {
            let payload = serde_json::to_vec(&item)
                .map_err(|e| NatsProtocolError::Run(format!("serialize prologue failed: {e}")))?;
            self.client
                .publish(reply.clone(), payload.into())
                .await
                .map_err(|e| NatsProtocolError::Run(format!("publish prologue failed: {e}")))?;
        }

        // Encode and publish each event
        while let Some(event) = rx.recv().await {
            for item in encoder.transcode(&event) {
                let payload = serde_json::to_vec(&item)
                    .map_err(|e| NatsProtocolError::Run(format!("serialize event failed: {e}")))?;
                self.client
                    .publish(reply.clone(), payload.into())
                    .await
                    .map_err(|e| NatsProtocolError::Run(format!("publish event failed: {e}")))?;
            }
        }

        // Emit epilogue
        for item in encoder.epilogue() {
            let payload = serde_json::to_vec(&item)
                .map_err(|e| NatsProtocolError::Run(format!("serialize epilogue failed: {e}")))?;
            self.client
                .publish(reply.clone(), payload.into())
                .await
                .map_err(|e| NatsProtocolError::Run(format!("publish epilogue failed: {e}")))?;
        }

        Ok(())
    }

    /// Publish a serialized error event to a NATS reply subject.
    pub async fn publish_error_event<ErrEvent: Serialize>(
        &self,
        reply: async_nats::Subject,
        event: ErrEvent,
    ) -> Result<(), NatsProtocolError> {
        let payload = serde_json::to_vec(&event)
            .map_err(|e| NatsProtocolError::Run(format!("serialize error event failed: {e}")))?
            .into();
        if let Err(publish_err) = self.client.publish(reply, payload).await {
            return Err(NatsProtocolError::Run(format!(
                "publish error event failed: {publish_err}"
            )));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nats_transport_config_default_buffer() {
        let config = NatsTransportConfig::default();
        assert_eq!(config.outbound_buffer, 64);
    }

    #[test]
    fn nats_protocol_error_display() {
        let err = NatsProtocolError::BadRequest("missing subject".to_string());
        assert_eq!(err.to_string(), "bad request: missing subject");

        let err = NatsProtocolError::Run("connection lost".to_string());
        assert_eq!(err.to_string(), "run error: connection lost");
    }

    #[test]
    fn nats_transport_config_custom_buffer() {
        let config = NatsTransportConfig {
            outbound_buffer: 128,
        };
        assert_eq!(config.outbound_buffer, 128);
    }
}
