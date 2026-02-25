use std::sync::Arc;

use async_trait::async_trait;
use futures::StreamExt;
use tirea_contract::Transcoder;
use tokio::sync::Mutex;

use crate::transport::{BoxStream, Endpoint, TransportError};

/// Bidirectional protocol transcoder endpoint.
///
/// Wraps an inner `Endpoint<R::Input, S::Output>` and presents
/// `Endpoint<R::Output, S::Input>`:
///
/// - **recv** (output direction): stateful stream transformation via
///   `R: Transcoder` — emits prologue, transcodes each item, then emits epilogue.
/// - **send** (input direction): per-item mapping via `S: Transcoder`.
pub struct TranscoderEndpoint<R, S>
where
    R: Transcoder,
    S: Transcoder,
{
    inner: Arc<dyn Endpoint<R::Input, S::Output>>,
    recv_transcoder: Mutex<Option<R>>,
    send_transcoder: Mutex<S>,
}

impl<R, S> TranscoderEndpoint<R, S>
where
    R: Transcoder + 'static,
    S: Transcoder + 'static,
{
    pub fn new(
        inner: Arc<dyn Endpoint<R::Input, S::Output>>,
        recv_transcoder: R,
        send_transcoder: S,
    ) -> Self {
        Self {
            inner,
            recv_transcoder: Mutex::new(Some(recv_transcoder)),
            send_transcoder: Mutex::new(send_transcoder),
        }
    }
}

#[async_trait]
impl<R, S> Endpoint<R::Output, S::Input> for TranscoderEndpoint<R, S>
where
    R: Transcoder + 'static,
    S: Transcoder + 'static,
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

    async fn send(&self, item: S::Input) -> Result<(), TransportError> {
        let mapped = {
            let mut transcoder = self.send_transcoder.lock().await;
            transcoder.transcode(&item)
        };
        for m in mapped {
            self.inner.send(m).await?;
        }
        Ok(())
    }

    async fn close(&self) -> Result<(), TransportError> {
        self.inner.close().await
    }
}
