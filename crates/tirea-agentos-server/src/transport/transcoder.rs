use std::sync::Arc;

use async_trait::async_trait;
use futures::StreamExt;
use tirea_contract::Transcoder;
use tokio::sync::Mutex;

use crate::transport::{BoxStream, Endpoint, TransportError};

/// Recv-only protocol transcoder endpoint.
///
/// Wraps an inner `Endpoint<R::Input, SendMsg>` and presents
/// `Endpoint<R::Output, SendMsg>`:
///
/// - **recv** (output direction): stateful stream transformation via
///   `R: Transcoder` — emits prologue, transcodes each item, then emits epilogue.
/// - **send** (input direction): passes through to the inner endpoint unchanged.
pub struct TranscoderEndpoint<R, SendMsg>
where
    R: Transcoder,
    SendMsg: Send + 'static,
{
    inner: Arc<dyn Endpoint<R::Input, SendMsg>>,
    recv_transcoder: Mutex<Option<R>>,
}

impl<R, SendMsg> TranscoderEndpoint<R, SendMsg>
where
    R: Transcoder + 'static,
    SendMsg: Send + 'static,
{
    pub fn new(
        inner: Arc<dyn Endpoint<R::Input, SendMsg>>,
        recv_transcoder: R,
    ) -> Self {
        Self {
            inner,
            recv_transcoder: Mutex::new(Some(recv_transcoder)),
        }
    }
}

#[async_trait]
impl<R, SendMsg> Endpoint<R::Output, SendMsg> for TranscoderEndpoint<R, SendMsg>
where
    R: Transcoder + 'static,
    SendMsg: Send + 'static,
{
    async fn recv(&self) -> Result<BoxStream<R::Output>, TransportError> {
        let recv_transcoder = self
            .recv_transcoder
            .lock()
            .await
            .take()
            .ok_or(TransportError::Closed)?;
        let inner_stream = self.inner.recv().await?;

        let stream = async_stream::stream! {
            let mut transcoder = recv_transcoder;
            let mut inner = inner_stream;

            for event in transcoder.prologue() {
                yield Ok(event);
            }

            while let Some(item) = inner.next().await {
                match item {
                    Ok(input) => {
                        for event in transcoder.transcode(&input) {
                            yield Ok(event);
                        }
                    }
                    Err(e) => {
                        yield Err(e);
                        return;
                    }
                }
            }

            for event in transcoder.epilogue() {
                yield Ok(event);
            }
        };

        Ok(Box::pin(stream))
    }

    async fn send(&self, item: SendMsg) -> Result<(), TransportError> {
        self.inner.send(item).await
    }

    async fn close(&self) -> Result<(), TransportError> {
        self.inner.close().await
    }
}
