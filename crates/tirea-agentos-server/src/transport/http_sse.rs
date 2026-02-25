use axum::body::Body;
use axum::http::{header, HeaderMap, HeaderValue};
use axum::response::{IntoResponse, Response};
use bytes::Bytes;
use serde::Serialize;
use std::convert::Infallible;
use std::marker::PhantomData;
use tirea_agentos::contracts::ToolCallDecision;
use tokio::sync::{broadcast, mpsc, Mutex};

use crate::transport::{BoxStream, Endpoint, TransportError};

pub struct HttpSseServerEndpoint<SendMsg> {
    ingress_rx: Mutex<Option<mpsc::UnboundedReceiver<ToolCallDecision>>>,
    sse_tx: mpsc::Sender<Bytes>,
    fanout: Option<broadcast::Sender<Bytes>>,
    _phantom: PhantomData<fn(SendMsg)>,
}

impl<SendMsg> HttpSseServerEndpoint<SendMsg> {
    pub fn new(
        ingress_rx: mpsc::UnboundedReceiver<ToolCallDecision>,
        sse_tx: mpsc::Sender<Bytes>,
    ) -> Self {
        Self {
            ingress_rx: Mutex::new(Some(ingress_rx)),
            sse_tx,
            fanout: None,
            _phantom: PhantomData,
        }
    }

    pub fn with_fanout(
        ingress_rx: mpsc::UnboundedReceiver<ToolCallDecision>,
        sse_tx: mpsc::Sender<Bytes>,
        fanout: broadcast::Sender<Bytes>,
    ) -> Self {
        Self {
            ingress_rx: Mutex::new(Some(ingress_rx)),
            sse_tx,
            fanout: Some(fanout),
            _phantom: PhantomData,
        }
    }
}

#[async_trait::async_trait]
impl<SendMsg> Endpoint<ToolCallDecision, SendMsg> for HttpSseServerEndpoint<SendMsg>
where
    SendMsg: Serialize + Send + 'static,
{
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

    async fn send(&self, item: SendMsg) -> Result<(), TransportError> {
        let json = serde_json::to_string(&item).map_err(|e| {
            tracing::warn!(error = %e, "failed to serialize SSE protocol event");
            TransportError::Io(format!("serialize event failed: {e}"))
        })?;
        let chunk = Bytes::from(format!("data: {json}\n\n"));
        if let Some(f) = &self.fanout {
            let _ = f.send(chunk.clone());
        }
        self.sse_tx
            .send(chunk)
            .await
            .map_err(|_| TransportError::Closed)
    }

    async fn close(&self) -> Result<(), TransportError> {
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
    use serde_json::json;

    #[tokio::test]
    async fn send_serializes_and_frames_as_sse() {
        let (_ingress_tx, ingress_rx) = mpsc::unbounded_channel::<ToolCallDecision>();
        let (sse_tx, mut sse_rx) = mpsc::channel::<Bytes>(4);
        let endpoint: HttpSseServerEndpoint<serde_json::Value> =
            HttpSseServerEndpoint::new(ingress_rx, sse_tx);

        let event = json!({"type": "test"});
        endpoint.send(event).await.unwrap();
        let received = sse_rx.recv().await.unwrap();
        assert_eq!(received, Bytes::from("data: {\"type\":\"test\"}\n\n"));
    }

    #[tokio::test]
    async fn send_with_fanout_broadcasts() {
        let (_ingress_tx, ingress_rx) = mpsc::unbounded_channel::<ToolCallDecision>();
        let (sse_tx, mut sse_rx) = mpsc::channel::<Bytes>(4);
        let (fanout_tx, mut fanout_rx) = broadcast::channel::<Bytes>(4);
        let endpoint: HttpSseServerEndpoint<serde_json::Value> =
            HttpSseServerEndpoint::with_fanout(ingress_rx, sse_tx, fanout_tx);

        let event = json!({"type": "test"});
        endpoint.send(event).await.unwrap();

        let received = sse_rx.recv().await.unwrap();
        let expected = Bytes::from("data: {\"type\":\"test\"}\n\n");
        assert_eq!(received, expected);

        let fanout_received = fanout_rx.recv().await.unwrap();
        assert_eq!(fanout_received, expected);
    }

    #[tokio::test]
    async fn send_returns_error_on_closed_channel() {
        let (_ingress_tx, ingress_rx) = mpsc::unbounded_channel::<ToolCallDecision>();
        let (sse_tx, sse_rx) = mpsc::channel::<Bytes>(4);
        let endpoint: HttpSseServerEndpoint<serde_json::Value> =
            HttpSseServerEndpoint::new(ingress_rx, sse_tx);
        drop(sse_rx);

        let result = endpoint.send(json!({"type": "test"})).await;
        assert!(result.is_err());
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
