use std::marker::PhantomData;
use std::sync::Arc;

use async_trait::async_trait;
use futures::StreamExt;
use tirea_agentos::contracts::AgentEvent;
use tirea_contract::ProtocolOutputEncoder;
use tokio::sync::Mutex;

use crate::transport::{BoxStream, Endpoint, TransportError};

/// Bidirectional protocol transcoder endpoint.
///
/// Wraps an inner `Endpoint<AgentEvent, InnerSend>` and presents
/// `Endpoint<E::Event, SendInput>`:
///
/// - **recv** (output direction): stateful stream transformation via
///   `ProtocolOutputEncoder` — emits prologue, transcodes each agent event,
///   then emits epilogue.
/// - **send** (input direction): stateless per-item mapping via
///   `Fn(SendInput) -> Result<InnerSend, TransportError>`.
pub struct TranscoderEndpoint<E, F, SendInput, InnerSend>
where
    E: ProtocolOutputEncoder<InputEvent = AgentEvent>,
{
    inner: Arc<dyn Endpoint<AgentEvent, InnerSend>>,
    encoder: Mutex<Option<E>>,
    send_mapper: F,
    _phantom: PhantomData<fn(SendInput)>,
}

impl<E, F, SendInput, InnerSend> TranscoderEndpoint<E, F, SendInput, InnerSend>
where
    E: ProtocolOutputEncoder<InputEvent = AgentEvent> + Send + 'static,
    E::Event: Send + 'static,
    F: Fn(SendInput) -> Result<InnerSend, TransportError> + Send + Sync + 'static,
    SendInput: Send + 'static,
    InnerSend: Send + 'static,
{
    pub fn new(
        inner: Arc<dyn Endpoint<AgentEvent, InnerSend>>,
        encoder: E,
        send_mapper: F,
    ) -> Self {
        Self {
            inner,
            encoder: Mutex::new(Some(encoder)),
            send_mapper,
            _phantom: PhantomData,
        }
    }
}

#[async_trait]
impl<E, F, SendInput, InnerSend> Endpoint<E::Event, SendInput>
    for TranscoderEndpoint<E, F, SendInput, InnerSend>
where
    E: ProtocolOutputEncoder<InputEvent = AgentEvent> + Send + 'static,
    E::Event: Send + 'static,
    F: Fn(SendInput) -> Result<InnerSend, TransportError> + Send + Sync + 'static,
    SendInput: Send + 'static,
    InnerSend: Send + 'static,
{
    async fn recv(&self) -> Result<BoxStream<E::Event>, TransportError> {
        let encoder = self
            .encoder
            .lock()
            .await
            .take()
            .ok_or(TransportError::Closed)?;
        let inner_stream = self.inner.recv().await?;

        let stream = async_stream::stream! {
            let mut encoder = encoder;
            let mut inner = inner_stream;

            for event in encoder.prologue() {
                yield Ok(event);
            }

            while let Some(item) = inner.next().await {
                match item {
                    Ok(agent_event) => {
                        for event in encoder.on_agent_event(&agent_event) {
                            yield Ok(event);
                        }
                    }
                    Err(e) => {
                        yield Err(e);
                        return;
                    }
                }
            }

            for event in encoder.epilogue() {
                yield Ok(event);
            }
        };

        Ok(Box::pin(stream))
    }

    async fn send(&self, item: SendInput) -> Result<(), TransportError> {
        let mapped = (self.send_mapper)(item)?;
        self.inner.send(mapped).await
    }

    async fn close(&self) -> Result<(), TransportError> {
        self.inner.close().await
    }
}
