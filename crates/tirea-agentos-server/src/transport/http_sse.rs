use axum::body::Body;
use axum::http::{header, HeaderMap, HeaderValue};
use axum::response::{IntoResponse, Response};
use bytes::Bytes;
use std::convert::Infallible;
use tirea_agentos::contracts::ToolCallDecision;
use tirea_agentos::runtime::loop_runner::RunCancellationToken;
use tokio::sync::{broadcast, mpsc, Mutex};

use crate::transport::{BoxStream, Endpoint, TransportError};

pub struct HttpSseUpstreamEndpoint {
    ingress_rx: Mutex<Option<mpsc::UnboundedReceiver<ToolCallDecision>>>,
    sse_tx: mpsc::Sender<Bytes>,
    fanout: Option<broadcast::Sender<Bytes>>,
    cancellation_token: RunCancellationToken,
}

impl HttpSseUpstreamEndpoint {
    pub fn new(
        ingress_rx: mpsc::UnboundedReceiver<ToolCallDecision>,
        sse_tx: mpsc::Sender<Bytes>,
        cancellation_token: RunCancellationToken,
    ) -> Self {
        Self {
            ingress_rx: Mutex::new(Some(ingress_rx)),
            sse_tx,
            fanout: None,
            cancellation_token,
        }
    }

    pub fn with_fanout(
        ingress_rx: mpsc::UnboundedReceiver<ToolCallDecision>,
        sse_tx: mpsc::Sender<Bytes>,
        fanout: broadcast::Sender<Bytes>,
        cancellation_token: RunCancellationToken,
    ) -> Self {
        Self {
            ingress_rx: Mutex::new(Some(ingress_rx)),
            sse_tx,
            fanout: Some(fanout),
            cancellation_token,
        }
    }
}

#[async_trait::async_trait]
impl Endpoint<ToolCallDecision, Bytes> for HttpSseUpstreamEndpoint {
    async fn recv(&self) -> Result<BoxStream<ToolCallDecision>, TransportError> {
        let mut guard = self.ingress_rx.lock().await;
        let mut rx = guard.take().ok_or(TransportError::Closed)?;
        let stream = async_stream::stream! {
            while let Some(item) = rx.recv().await {
                yield Ok(item);
            }
        };
        Ok(Box::pin(stream))
    }

    async fn send(&self, item: Bytes) -> Result<(), TransportError> {
        if let Some(f) = &self.fanout {
            let _ = f.send(item.clone());
        }
        self.sse_tx.send(item).await.map_err(|_| {
            self.cancellation_token.cancel();
            TransportError::Closed
        })
    }

    async fn close(&self) -> Result<(), TransportError> {
        self.cancellation_token.cancel();
        Ok(())
    }
}

pub fn sse_body_stream(
    mut rx: mpsc::Receiver<Bytes>,
) -> impl futures::Stream<Item = Result<Bytes, Infallible>> + Send + 'static {
    async_stream::stream! {
        while let Some(chunk) = rx.recv().await {
            yield Ok::<Bytes, Infallible>(chunk);
        }
    }
}

pub fn sse_response<S>(stream: S) -> Response
where
    S: futures::Stream<Item = Result<Bytes, Infallible>> + Send + 'static,
{
    let mut headers = HeaderMap::new();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/event-stream"),
    );
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
    headers.insert(header::CONNECTION, HeaderValue::from_static("keep-alive"));
    (headers, Body::from_stream(stream)).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::StreamExt;

    #[tokio::test]
    async fn send_without_fanout() {
        let (_ingress_tx, ingress_rx) = mpsc::unbounded_channel::<ToolCallDecision>();
        let (sse_tx, mut sse_rx) = mpsc::channel::<Bytes>(4);
        let token = RunCancellationToken::new();
        let endpoint = HttpSseUpstreamEndpoint::new(ingress_rx, sse_tx, token.clone());

        let data = Bytes::from("data: test\n\n");
        endpoint.send(data.clone()).await.unwrap();
        let received = sse_rx.recv().await.unwrap();
        assert_eq!(received, data);
        assert!(!token.is_cancelled());
    }

    #[tokio::test]
    async fn send_with_fanout() {
        let (_ingress_tx, ingress_rx) = mpsc::unbounded_channel::<ToolCallDecision>();
        let (sse_tx, mut sse_rx) = mpsc::channel::<Bytes>(4);
        let (fanout_tx, mut fanout_rx) = broadcast::channel::<Bytes>(4);
        let token = RunCancellationToken::new();
        let endpoint =
            HttpSseUpstreamEndpoint::with_fanout(ingress_rx, sse_tx, fanout_tx, token.clone());

        let data = Bytes::from("data: test\n\n");
        endpoint.send(data.clone()).await.unwrap();

        let received = sse_rx.recv().await.unwrap();
        assert_eq!(received, data);

        let fanout_received = fanout_rx.recv().await.unwrap();
        assert_eq!(fanout_received, data);
    }

    #[tokio::test]
    async fn send_cancels_token_on_closed_channel() {
        let (_ingress_tx, ingress_rx) = mpsc::unbounded_channel::<ToolCallDecision>();
        let (sse_tx, sse_rx) = mpsc::channel::<Bytes>(4);
        let token = RunCancellationToken::new();
        let endpoint = HttpSseUpstreamEndpoint::new(ingress_rx, sse_tx, token.clone());
        drop(sse_rx);

        let result = endpoint.send(Bytes::from("data: test\n\n")).await;
        assert!(result.is_err());
        assert!(token.is_cancelled());
    }

    #[tokio::test]
    async fn sse_body_stream_yields_all_chunks() {
        let (tx, rx) = mpsc::channel::<Bytes>(4);
        let stream = sse_body_stream(rx);
        tokio::pin!(stream);

        tx.send(Bytes::from("a")).await.unwrap();
        tx.send(Bytes::from("b")).await.unwrap();
        drop(tx);

        let items: Vec<Bytes> = stream.map(|r| r.unwrap()).collect().await;
        assert_eq!(items, vec![Bytes::from("a"), Bytes::from("b")]);
    }
}
