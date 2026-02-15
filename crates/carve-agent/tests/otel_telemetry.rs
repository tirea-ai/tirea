use carve_agent::contracts::agent_plugin::AgentPlugin;
use carve_agent::contracts::traits::tool::{Tool, ToolDescriptor, ToolError, ToolResult};
use carve_agent::extensions::observability::{InMemorySink, LLMMetryPlugin};
use carve_agent::prelude::Context;
use carve_agent::runtime::loop_runner::{
    execute_tools_with_plugins, run_loop_stream, run_step, AgentConfig,
};
use carve_agent::runtime::streaming::{AgentEvent, StreamResult};
use carve_agent::thread::Thread;
use carve_agent::types::{Message, ToolCall};
use futures::StreamExt;
use opentelemetry::trace::TracerProvider as _;
use opentelemetry_sdk::trace::{InMemorySpanExporter, SdkTracerProvider, SpanData};
use serde_json::json;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tracing_opentelemetry::OpenTelemetryLayer;
use tracing_subscriber::layer::SubscriberExt;

fn find_attribute<'a>(span: &'a SpanData, key: &str) -> Option<&'a opentelemetry::Value> {
    span.attributes
        .iter()
        .find(|kv| kv.key.as_str() == key)
        .map(|kv| &kv.value)
}

fn setup_otel_test() -> (
    tracing::subscriber::DefaultGuard,
    InMemorySpanExporter,
    SdkTracerProvider,
) {
    let exporter = InMemorySpanExporter::default();
    let provider = SdkTracerProvider::builder()
        .with_simple_exporter(exporter.clone())
        .build();
    let tracer = provider.tracer("test");
    let otel_layer = OpenTelemetryLayer::new(tracer);
    let subscriber = tracing_subscriber::registry::Registry::default().with(otel_layer);
    let guard = tracing::subscriber::set_default(subscriber);
    (guard, exporter, provider)
}

async fn start_single_response_server(
    status: &str,
    content_type: &str,
    body: &'static str,
) -> (String, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Length: {}\r\nContent-Type: {content_type}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
    .into_bytes();

    let handle = tokio::spawn(async move {
        // Serve at most one request for this test.
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut buf = [0u8; 4096];
        let _ = socket.read(&mut buf).await;
        let _ = socket.write_all(&response).await;
        let _ = socket.shutdown().await;
    });

    // OpenAI adapter expects base_url ending with `/v1/` so it can join `chat/completions`.
    let base_url = format!("http://{addr}/v1/");
    (base_url, handle)
}

async fn start_sse_server(events: Vec<&'static str>) -> (String, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
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
    (base_url, handle)
}

struct NoopTool {
    id: &'static str,
}

#[async_trait::async_trait]
impl Tool for NoopTool {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor::new(self.id, self.id, "noop")
    }

    async fn execute(
        &self,
        _args: serde_json::Value,
        _ctx: &Context<'_>,
    ) -> Result<ToolResult, ToolError> {
        Ok(ToolResult::success(self.id, json!({"ok": true})))
    }
}

#[tokio::test(flavor = "current_thread")]
async fn test_execute_tools_parallel_exports_distinct_otel_tool_spans() {
    let (_guard, exporter, provider) = setup_otel_test();

    let sink = InMemorySink::new();
    let plugin =
        Arc::new(LLMMetryPlugin::new(sink).with_provider("test-provider")) as Arc<dyn AgentPlugin>;

    let thread = Thread::with_initial_state("t", json!({})).with_message(Message::user("hi"));
    let result = StreamResult {
        text: "tools".into(),
        tool_calls: vec![
            ToolCall::new("c1", "t1", json!({})),
            ToolCall::new("c2", "t2", json!({})),
            ToolCall::new("c3", "t3", json!({})),
        ],
        usage: None,
    };

    let mut tools: HashMap<String, Arc<dyn Tool>> = HashMap::new();
    tools.insert("t1".into(), Arc::new(NoopTool { id: "t1" }));
    tools.insert("t2".into(), Arc::new(NoopTool { id: "t2" }));
    tools.insert("t3".into(), Arc::new(NoopTool { id: "t3" }));

    let _session = execute_tools_with_plugins(thread, &result, &tools, true, &[plugin])
        .await
        .unwrap();

    let _ = provider.force_flush();
    let exported = exporter.get_finished_spans().unwrap();
    let tool_spans: Vec<&SpanData> = exported
        .iter()
        .filter(|s| s.name.starts_with("execute_tool "))
        .collect();
    assert_eq!(tool_spans.len(), 3);

    let mut seen: Vec<(String, String)> = tool_spans
        .iter()
        .map(|s| {
            let tool = find_attribute(s, "gen_ai.tool.name")
                .unwrap()
                .as_str()
                .to_string();
            let call_id = find_attribute(s, "gen_ai.tool.call.id")
                .unwrap()
                .as_str()
                .to_string();
            (tool, call_id)
        })
        .collect();
    seen.sort();
    assert_eq!(
        seen,
        vec![
            ("t1".to_string(), "c1".to_string()),
            ("t2".to_string(), "c2".to_string()),
            ("t3".to_string(), "c3".to_string()),
        ]
    );
}

#[tokio::test(flavor = "current_thread")]
async fn test_run_step_non_streaming_propagates_usage_and_exports_tokens_to_otel() {
    let (base_url, _server) = start_single_response_server(
        "200 OK",
        "application/json",
        r#"{"model":"gpt-4","usage":{"prompt_tokens":10,"completion_tokens":5,"total_tokens":15},"choices":[{"message":{"content":"hi"}}]}"#,
    )
    .await;

    let (_guard, exporter, provider) = setup_otel_test();

    let sink = InMemorySink::new();
    let plugin = Arc::new(
        LLMMetryPlugin::new(sink.clone())
            .with_model("gpt-4")
            .with_provider("test-provider"),
    ) as Arc<dyn AgentPlugin>;

    let client = genai::Client::builder()
        .with_service_target_resolver_fn(move |mut t: genai::ServiceTarget| {
            t.endpoint = genai::resolver::Endpoint::from_owned(base_url.clone());
            t.auth = genai::resolver::AuthData::from_single("test-key");
            Ok(t)
        })
        .build();

    let config = AgentConfig::new("gpt-4").with_plugin(plugin);
    let thread = Thread::with_initial_state("s", json!({})).with_message(Message::user("hi"));
    let tools: HashMap<String, Arc<dyn Tool>> = HashMap::new();

    let (_thread, result) = run_step(&client, &config, thread, &tools).await.unwrap();
    let usage = result
        .usage
        .expect("expected usage in non-streaming result");
    assert_eq!(usage.prompt_tokens, Some(10));
    assert_eq!(usage.completion_tokens, Some(5));
    assert_eq!(usage.total_tokens, Some(15));

    let _ = provider.force_flush();
    let exported = exporter.get_finished_spans().unwrap();
    let span = exported
        .iter()
        .find(|s| s.name.starts_with("chat "))
        .expect("expected chat span");
    assert_eq!(
        find_attribute(span, "gen_ai.usage.input_tokens"),
        Some(&opentelemetry::Value::I64(10))
    );
    assert_eq!(
        find_attribute(span, "gen_ai.usage.output_tokens"),
        Some(&opentelemetry::Value::I64(5))
    );
}

#[tokio::test(flavor = "current_thread")]
async fn test_run_step_llm_error_closes_inference_span_and_sets_error_type() {
    // Return invalid JSON so genai parsing fails deterministically.
    let (base_url, _server) = start_single_response_server("200 OK", "application/json", "{").await;

    let (_guard, exporter, provider) = setup_otel_test();

    let sink = InMemorySink::new();
    let plugin = Arc::new(
        LLMMetryPlugin::new(sink.clone())
            .with_model("gpt-4")
            .with_provider("test-provider"),
    ) as Arc<dyn AgentPlugin>;

    let client = genai::Client::builder()
        .with_service_target_resolver_fn(move |mut t: genai::ServiceTarget| {
            t.endpoint = genai::resolver::Endpoint::from_owned(base_url.clone());
            t.auth = genai::resolver::AuthData::from_single("test-key");
            Ok(t)
        })
        .build();

    let config = AgentConfig::new("gpt-4").with_plugin(plugin);
    let thread = Thread::with_initial_state("s", json!({})).with_message(Message::user("hi"));
    let tools: HashMap<String, Arc<dyn Tool>> = HashMap::new();

    let err = run_step(&client, &config, thread, &tools)
        .await
        .err()
        .unwrap();
    assert!(err.to_string().contains("LLM error"));

    // Metrics should record the failed inference.
    let m = sink.metrics();
    assert_eq!(m.inference_count(), 1);
    assert_eq!(
        m.inferences[0].error_type.as_deref(),
        Some("llm_exec_error")
    );

    // OTel export should include error.type and Error status.
    let _ = provider.force_flush();
    let exported = exporter.get_finished_spans().unwrap();
    let span = exported
        .iter()
        .find(|s| s.name.starts_with("chat "))
        .expect("expected chat span");
    let error_type = find_attribute(span, "error.type")
        .map(|v| v.as_str().to_string())
        .unwrap_or_default();
    assert_eq!(error_type, "llm_exec_error");
    assert!(matches!(
        span.status,
        opentelemetry::trace::Status::Error { .. }
    ));
}

#[tokio::test(flavor = "current_thread")]
async fn test_run_loop_stream_http_error_closes_inference_span() {
    let (base_url, _server) = start_single_response_server(
        "500 Internal Server Error",
        "application/json",
        r#"{"error":"fail"}"#,
    )
    .await;

    let (_guard, exporter, provider) = setup_otel_test();

    let sink = InMemorySink::new();
    let plugin = Arc::new(
        LLMMetryPlugin::new(sink.clone())
            .with_model("gpt-4")
            .with_provider("test-provider"),
    ) as Arc<dyn AgentPlugin>;

    let client = genai::Client::builder()
        .with_service_target_resolver_fn(move |mut t: genai::ServiceTarget| {
            t.endpoint = genai::resolver::Endpoint::from_owned(base_url.clone());
            t.auth = genai::resolver::AuthData::from_single("test-key");
            Ok(t)
        })
        .build();

    let config = AgentConfig::new("gpt-4").with_plugin(plugin);
    let thread = Thread::with_initial_state("s", json!({})).with_message(Message::user("hi"));
    let tools: HashMap<String, Arc<dyn Tool>> = HashMap::new();

    let events: Vec<_> = run_loop_stream(client, config, thread, tools, Default::default())
        .collect()
        .await;
    assert!(events.iter().any(|e| matches!(e, AgentEvent::Error { .. })));

    // Metrics should record a failed inference.
    let m = sink.metrics();
    assert_eq!(m.inference_count(), 1);
    assert_eq!(
        m.inferences[0].error_type.as_deref(),
        Some("llm_stream_event_error")
    );

    let _ = provider.force_flush();
    let exported = exporter.get_finished_spans().unwrap();
    let span = exported
        .iter()
        .find(|s| s.name.starts_with("chat "))
        .expect("expected chat span");
    let error_type = find_attribute(span, "error.type")
        .map(|v| v.as_str().to_string())
        .unwrap_or_default();
    assert_eq!(error_type, "llm_stream_event_error");
    assert!(matches!(
        span.status,
        opentelemetry::trace::Status::Error { .. }
    ));
}

#[tokio::test(flavor = "current_thread")]
async fn test_run_loop_stream_parse_error_closes_inference_span() {
    // First event is valid; second event is invalid JSON to trigger StreamParse.
    let (base_url, _server) = start_sse_server(vec![
        "data: {\"choices\":[{\"delta\":{\"content\":\"hi\"}}]}\n\n",
        "data: {invalid-json}\n\n",
    ])
    .await;

    let (_guard, exporter, provider) = setup_otel_test();

    let sink = InMemorySink::new();
    let plugin = Arc::new(
        LLMMetryPlugin::new(sink.clone())
            .with_model("gpt-4")
            .with_provider("test-provider"),
    ) as Arc<dyn AgentPlugin>;

    let client = genai::Client::builder()
        .with_service_target_resolver_fn(move |mut t: genai::ServiceTarget| {
            t.endpoint = genai::resolver::Endpoint::from_owned(base_url.clone());
            t.auth = genai::resolver::AuthData::from_single("test-key");
            Ok(t)
        })
        .build();

    let config = AgentConfig::new("gpt-4").with_plugin(plugin);
    let thread = Thread::with_initial_state("s", json!({})).with_message(Message::user("hi"));
    let tools: HashMap<String, Arc<dyn Tool>> = HashMap::new();

    let events: Vec<_> = run_loop_stream(client, config, thread, tools, Default::default())
        .collect()
        .await;
    assert!(events.iter().any(|e| matches!(e, AgentEvent::Error { .. })));
    assert!(events
        .iter()
        .any(|e| matches!(e, AgentEvent::TextDelta { .. })));

    let m = sink.metrics();
    assert_eq!(m.inference_count(), 1);
    assert_eq!(
        m.inferences[0].error_type.as_deref(),
        Some("llm_stream_event_error")
    );

    let _ = provider.force_flush();
    let exported = exporter.get_finished_spans().unwrap();
    let span = exported
        .iter()
        .find(|s| s.name.starts_with("chat "))
        .expect("expected chat span");
    let error_type = find_attribute(span, "error.type")
        .map(|v| v.as_str().to_string())
        .unwrap_or_default();
    assert_eq!(error_type, "llm_stream_event_error");
    assert!(matches!(
        span.status,
        opentelemetry::trace::Status::Error { .. }
    ));
}
