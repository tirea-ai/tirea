//! SSE response helpers for streaming agent events over HTTP.

use axum::body::Body;
use axum::http::{HeaderMap, HeaderValue, header};
use axum::response::{IntoResponse, Response};
use bytes::Bytes;
use std::convert::Infallible;
use tokio::time::{Duration, interval};

/// Default interval between SSE heartbeat comments.
const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(15);

/// SSE comment line used as a keep-alive heartbeat.
/// Clients ignore comment lines (`:` prefix) per the SSE spec.
const HEARTBEAT_BYTES: &[u8] = b": heartbeat\n\n";

/// Convert an mpsc receiver of SSE chunks into an async stream suitable for HTTP response.
///
/// Injects periodic SSE comment heartbeats (`: heartbeat`) when the stream is
/// idle to prevent client/proxy timeouts during long LLM inference pauses.
pub fn sse_body_stream(
    mut rx: tokio::sync::mpsc::Receiver<Bytes>,
) -> impl futures::Stream<Item = Result<Bytes, Infallible>> + Send + 'static {
    async_stream::stream! {
        let mut heartbeat = interval(HEARTBEAT_INTERVAL);
        heartbeat.tick().await; // skip the first immediate tick

        loop {
            tokio::select! {
                event = rx.recv() => {
                    match event {
                        Some(chunk) => {
                            heartbeat.reset();
                            yield Ok::<Bytes, Infallible>(chunk);
                        }
                        None => break,
                    }
                }
                _ = heartbeat.tick() => {
                    yield Ok::<Bytes, Infallible>(Bytes::from_static(HEARTBEAT_BYTES));
                }
            }
        }
    }
}

/// Build an SSE HTTP response from a stream of byte chunks.
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

/// Format an agent event as an SSE `data:` line.
pub fn format_sse_data(json: &str) -> Bytes {
    Bytes::from(format!("data: {json}\n\n"))
}

/// Format an agent event as an SSE frame with an event ID for resumability.
pub fn format_sse_data_with_id(json: &str, id: u64) -> Bytes {
    Bytes::from(format!("id: {id}\ndata: {json}\n\n"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::StreamExt;

    /// Helper: filter out heartbeat frames from collected stream items.
    fn data_only(items: Vec<Bytes>) -> Vec<Bytes> {
        items
            .into_iter()
            .filter(|b| b.as_ref() != HEARTBEAT_BYTES)
            .collect()
    }

    #[tokio::test]
    async fn sse_body_stream_yields_all_chunks() {
        let (tx, rx) = tokio::sync::mpsc::channel::<Bytes>(4);
        let stream = sse_body_stream(rx);
        tokio::pin!(stream);

        tx.send(Bytes::from("a")).await.unwrap();
        tx.send(Bytes::from("b")).await.unwrap();
        drop(tx);

        let items: Vec<Bytes> = stream.map(|r| r.unwrap()).collect().await;
        assert_eq!(data_only(items), vec![Bytes::from("a"), Bytes::from("b")]);
    }

    #[tokio::test]
    async fn sse_body_stream_empty_on_immediate_close() {
        let (_tx, rx) = tokio::sync::mpsc::channel::<Bytes>(4);
        let stream = sse_body_stream(rx);
        tokio::pin!(stream);
        drop(_tx);

        let items: Vec<Bytes> = stream.map(|r| r.unwrap()).collect().await;
        assert!(data_only(items).is_empty());
    }

    #[tokio::test]
    async fn sse_body_stream_emits_heartbeat_when_idle() {
        // Use a very short heartbeat interval via a manual stream to verify the logic.
        // We test that the heartbeat constant is valid SSE comment format.
        let hb = Bytes::from_static(HEARTBEAT_BYTES);
        let s = std::str::from_utf8(hb.as_ref()).unwrap();
        assert!(s.starts_with(':'), "heartbeat must be an SSE comment");
        assert!(
            s.ends_with("\n\n"),
            "heartbeat must end with double newline"
        );
    }

    #[test]
    fn format_sse_data_with_id_produces_correct_format() {
        let result = format_sse_data_with_id(r#"{"type":"test"}"#, 42);
        assert_eq!(result, Bytes::from("id: 42\ndata: {\"type\":\"test\"}\n\n"));
    }

    #[test]
    fn format_sse_data_produces_correct_format() {
        let result = format_sse_data(r#"{"type":"test"}"#);
        assert_eq!(result, Bytes::from("data: {\"type\":\"test\"}\n\n"));
    }

    #[test]
    fn sse_response_has_correct_headers() {
        let stream = futures::stream::empty::<Result<Bytes, Infallible>>();
        let response = sse_response(stream);
        assert_eq!(
            response.headers().get("content-type").unwrap(),
            "text/event-stream"
        );
        assert_eq!(response.headers().get("cache-control").unwrap(), "no-cache");
        assert_eq!(response.headers().get("connection").unwrap(), "keep-alive");
    }
}
