#![allow(missing_docs)]

use opentelemetry::trace::TracerProvider as _;
use opentelemetry_otlp::WithExportConfig;
use opentelemetry_sdk::trace::SdkTracerProvider;
use serde_json::Value;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tracing_opentelemetry::OpenTelemetryLayer;
use tracing_subscriber::layer::SubscriberExt;

/// Set up an OTel subscriber that exports spans to Phoenix via OTLP/HTTP.
pub fn setup_otel_to_phoenix(
    otlp_traces_endpoint: &str,
    tracer_name: &str,
) -> (tracing::subscriber::DefaultGuard, SdkTracerProvider) {
    let exporter = opentelemetry_otlp::SpanExporter::builder()
        .with_http()
        .with_endpoint(otlp_traces_endpoint.to_string())
        .build()
        .expect("failed to build OTLP exporter");

    let provider = SdkTracerProvider::builder()
        .with_batch_exporter(exporter)
        .build();

    let tracer = provider.tracer(tracer_name.to_string());
    let otel_layer = OpenTelemetryLayer::new(tracer);
    let subscriber = tracing_subscriber::registry::Registry::default().with(otel_layer);
    let guard = tracing::subscriber::set_default(subscriber);
    (guard, provider)
}

/// Start a mock SSE (Server-Sent Events) HTTP server that streams the given events.
pub async fn start_sse_server(
    events: Vec<&'static str>,
) -> std::io::Result<(String, tokio::task::JoinHandle<()>)> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr().unwrap();

    let handle = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut buf = [0u8; 4096];
        let _ = socket.read(&mut buf).await;

        let headers = b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-cache\r\nConnection: close\r\n\r\n";
        let _ = socket.write_all(headers).await;

        for ev in events {
            let _ = socket.write_all(ev.as_bytes()).await;
            let _ = socket.flush().await;
            tokio::time::sleep(tokio::time::Duration::from_millis(5)).await;
        }

        let _ = socket.shutdown().await;
    });

    let base_url = format!("http://{addr}/v1/");
    Ok((base_url, handle))
}

/// Start a mock HTTP server that returns a single fixed response.
pub async fn start_single_response_server(
    status: &str,
    content_type: &str,
    body: &'static str,
) -> std::io::Result<(String, tokio::task::JoinHandle<()>)> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr().unwrap();

    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Length: {}\r\nContent-Type: {content_type}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
    .into_bytes();

    let handle = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut buf = [0u8; 4096];
        let _ = socket.read(&mut buf).await;
        let _ = socket.write_all(&response).await;
        let _ = socket.shutdown().await;
    });

    let base_url = format!("http://{addr}/v1/");
    Ok((base_url, handle))
}

/// Poll Phoenix REST API until a span matching `predicate` appears or timeout.
pub async fn wait_for_span<F>(project_spans_url: &str, mut predicate: F) -> Option<Value>
where
    F: FnMut(&Value) -> bool,
{
    let client = reqwest::Client::new();
    for _ in 0..50 {
        let response = client.get(project_spans_url).send().await.ok()?;
        if response.status().is_success() {
            let payload: Value = response.json().await.ok()?;
            if let Some(spans) = payload.get("data").and_then(Value::as_array) {
                for span in spans {
                    if predicate(span) {
                        return Some(span.clone());
                    }
                }
            }
        }
        tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;
    }
    None
}

/// Poll Phoenix for a chat span matching the given model name.
pub async fn wait_for_chat_span(project_spans_url: &str, model: &str) -> Option<Value> {
    wait_for_span(project_spans_url, |span| {
        let is_chat = span
            .get("name")
            .and_then(Value::as_str)
            .is_some_and(|name| name.starts_with("chat "));
        let model_ok = attr_str(span, "gen_ai.request.model") == Some(model);
        is_chat && model_ok
    })
    .await
}

/// Poll Phoenix for a span whose `gen_ai.request.model` matches.
pub async fn wait_for_span_with_model(project_spans_url: &str, model: &str) -> Option<Value> {
    wait_for_span(project_spans_url, |span| {
        attr_str(span, "gen_ai.request.model") == Some(model)
    })
    .await
}

/// Assert that Phoenix is reachable and healthy.
pub async fn ensure_phoenix_healthy(phoenix_base_url: &str) {
    let health_check = reqwest::Client::new()
        .get(format!("{phoenix_base_url}/v1/projects"))
        .send()
        .await
        .unwrap_or_else(|e| panic!("Phoenix server is not reachable at {phoenix_base_url}: {e}"));
    assert!(
        health_check.status().is_success(),
        "Phoenix health check failed at {phoenix_base_url}/v1/projects with status {}",
        health_check.status()
    );
}

/// Extract a string attribute from a Phoenix span JSON.
pub fn attr_str<'a>(span: &'a Value, key: &str) -> Option<&'a str> {
    span.get("attributes")
        .and_then(Value::as_object)
        .and_then(|attrs| attrs.get(key))
        .and_then(Value::as_str)
}

/// Read Phoenix connection parameters from environment with defaults.
pub struct PhoenixConfig {
    pub base_url: String,
    pub project_spans_url: String,
    pub otlp_traces_endpoint: String,
}

impl PhoenixConfig {
    pub fn from_env() -> Self {
        let base_url = std::env::var("PHOENIX_BASE_URL")
            .unwrap_or_else(|_| "http://127.0.0.1:6006".to_string());
        let project_spans_url = format!("{base_url}/v1/projects/default/spans?limit=500");
        let otlp_traces_endpoint = std::env::var("PHOENIX_OTLP_TRACES_ENDPOINT")
            .unwrap_or_else(|_| "http://127.0.0.1:6006/v1/traces".to_string());
        Self {
            base_url,
            project_spans_url,
            otlp_traces_endpoint,
        }
    }
}

/// Generate a unique timestamp-based suffix for test isolation.
pub fn unique_suffix() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system clock before unix epoch")
        .as_millis()
}
