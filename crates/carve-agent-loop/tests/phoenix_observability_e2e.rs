#![allow(missing_docs)]

use carve_agent_extension_observability::{InMemorySink, LLMMetryPlugin};
use carve_agent_loop::contracts::plugin::AgentPlugin;
use carve_agent_loop::contracts::runtime::{AgentEvent, StreamResult};
use carve_agent_loop::contracts::state::{AgentState, Message, ToolCall};
use carve_agent_loop::contracts::tool::{Tool, ToolDescriptor, ToolError, ToolResult};
use carve_agent_loop::contracts::ToolCallContext;
use carve_agent_loop::runtime::loop_runner::{
    execute_tools_with_plugins, run_loop_stream, run_step, AgentConfig,
};
use futures::StreamExt;
use phoenix_test_helpers::{
    attr_str, ensure_phoenix_healthy, start_single_response_server, start_sse_server,
    setup_otel_to_phoenix, unique_suffix, wait_for_chat_span, wait_for_span, PhoenixConfig,
};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Debug)]
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
        _args: Value,
        _ctx: &ToolCallContext<'_>,
    ) -> Result<ToolResult, ToolError> {
        Ok(ToolResult::success(self.id, json!({"ok": true})))
    }
}

#[derive(Debug)]
struct FailingTool {
    id: &'static str,
}

#[async_trait::async_trait]
impl Tool for FailingTool {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor::new(self.id, self.id, "always fails")
    }

    async fn execute(
        &self,
        _args: Value,
        _ctx: &ToolCallContext<'_>,
    ) -> Result<ToolResult, ToolError> {
        Err(ToolError::ExecutionFailed(format!("{} exploded", self.id)))
    }
}

#[tokio::test(flavor = "current_thread")]
async fn test_llmmetry_exports_to_phoenix_via_otlp() {
    let cfg = PhoenixConfig::from_env();
    ensure_phoenix_healthy(&cfg.base_url).await;

    let (base_url, _server) = start_single_response_server(
        "200 OK",
        "application/json",
        r#"{"model":"gpt-4","usage":{"prompt_tokens":10,"completion_tokens":5,"total_tokens":15},"choices":[{"message":{"content":"hi"}}]}"#,
    )
    .await
    .unwrap_or_else(|e| panic!("failed to start local mock LLM server for Phoenix e2e: {e}"));

    let (_guard, provider) = setup_otel_to_phoenix(&cfg.otlp_traces_endpoint, "phoenix-loop-e2e");
    let model_name = format!("phoenix-loop-e2e-{}", unique_suffix());

    let sink = InMemorySink::new();
    let plugin = Arc::new(
        LLMMetryPlugin::new(sink)
            .with_model(model_name.clone())
            .with_provider("test-provider"),
    ) as Arc<dyn AgentPlugin>;

    let client = genai::Client::builder()
        .with_service_target_resolver_fn(move |mut t: genai::ServiceTarget| {
            t.endpoint = genai::resolver::Endpoint::from_owned(base_url.clone());
            t.auth = genai::resolver::AuthData::from_single("test-key");
            Ok(t)
        })
        .build();

    let config = AgentConfig::new(model_name.clone()).with_plugin(plugin);
    let state = AgentState::with_initial_state("phoenix-e2e-state", json!({}))
        .with_message(Message::user("hi"));
    let tools: HashMap<String, Arc<dyn Tool>> = HashMap::new();

    let _ = run_step(&client, &config, state, &tools)
        .await
        .expect("run_step should succeed");

    let _ = provider.force_flush();

    let span = wait_for_chat_span(&cfg.project_spans_url, &model_name)
        .await
        .expect("expected at least one chat span exported to Phoenix");

    let attributes = span
        .get("attributes")
        .and_then(Value::as_object)
        .expect("expected span attributes");

    assert!(
        attributes.keys().any(|k| k.starts_with("gen_ai.")),
        "expected at least one gen_ai.* attribute"
    );
    assert_eq!(
        attributes
            .get("gen_ai.request.model")
            .and_then(Value::as_str),
        Some(model_name.as_str())
    );
}

#[tokio::test(flavor = "current_thread")]
async fn test_llmmetry_exports_error_span_to_phoenix_via_otlp() {
    let cfg = PhoenixConfig::from_env();
    ensure_phoenix_healthy(&cfg.base_url).await;

    let (base_url, _server) = start_single_response_server("200 OK", "application/json", "{")
        .await
        .unwrap_or_else(|e| panic!("failed to start local mock LLM server for Phoenix e2e: {e}"));

    let (_guard, provider) = setup_otel_to_phoenix(&cfg.otlp_traces_endpoint, "phoenix-loop-e2e");
    let model_name = format!("phoenix-loop-e2e-err-{}", unique_suffix());

    let sink = InMemorySink::new();
    let plugin = Arc::new(
        LLMMetryPlugin::new(sink)
            .with_model(model_name.clone())
            .with_provider("test-provider"),
    ) as Arc<dyn AgentPlugin>;

    let client = genai::Client::builder()
        .with_service_target_resolver_fn(move |mut t: genai::ServiceTarget| {
            t.endpoint = genai::resolver::Endpoint::from_owned(base_url.clone());
            t.auth = genai::resolver::AuthData::from_single("test-key");
            Ok(t)
        })
        .build();

    let config = AgentConfig::new(model_name.clone()).with_plugin(plugin);
    let state = AgentState::with_initial_state("phoenix-e2e-state-err", json!({}))
        .with_message(Message::user("hi"));
    let tools: HashMap<String, Arc<dyn Tool>> = HashMap::new();

    let err = run_step(&client, &config, state, &tools)
        .await
        .err()
        .expect("run_step should fail for invalid LLM json");
    assert!(err.to_string().contains("LLM error"));

    let _ = provider.force_flush();

    let span = wait_for_chat_span(&cfg.project_spans_url, &model_name)
        .await
        .expect("expected error chat span exported to Phoenix");
    assert_eq!(
        attr_str(&span, "error.type"),
        Some("llm_exec_error"),
        "expected llm_exec_error on exported span"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn test_llmmetry_exports_tool_spans_to_phoenix_via_otlp() {
    let cfg = PhoenixConfig::from_env();
    ensure_phoenix_healthy(&cfg.base_url).await;

    let (_guard, provider) =
        setup_otel_to_phoenix(&cfg.otlp_traces_endpoint, "phoenix-loop-e2e");
    let provider_name = format!("phoenix-tool-provider-{}", unique_suffix());

    let sink = InMemorySink::new();
    let plugin = Arc::new(LLMMetryPlugin::new(sink).with_provider(provider_name.clone()))
        as Arc<dyn AgentPlugin>;

    let thread = AgentState::with_initial_state("phoenix-tool-span-state", json!({}))
        .with_message(Message::user("hi"));
    let result = StreamResult {
        text: "tools".into(),
        tool_calls: vec![
            ToolCall::new("phoenix_call_1", "phoenix_t1", json!({})),
            ToolCall::new("phoenix_call_2", "phoenix_t2", json!({})),
        ],
        usage: None,
    };

    let mut tools: HashMap<String, Arc<dyn Tool>> = HashMap::new();
    tools.insert(
        "phoenix_t1".to_string(),
        Arc::new(NoopTool { id: "phoenix_t1" }),
    );
    tools.insert(
        "phoenix_t2".to_string(),
        Arc::new(NoopTool { id: "phoenix_t2" }),
    );

    let _ = execute_tools_with_plugins(thread, &result, &tools, true, &[plugin])
        .await
        .expect("tool execution with observability plugin should succeed");

    let _ = provider.force_flush();

    for call_id in ["phoenix_call_1", "phoenix_call_2"] {
        let span = wait_for_span(&cfg.project_spans_url, |span| {
            span.get("name")
                .and_then(Value::as_str)
                .is_some_and(|name| name.starts_with("execute_tool "))
                && attr_str(span, "gen_ai.provider.name") == Some(provider_name.as_str())
                && attr_str(span, "gen_ai.tool.call.id") == Some(call_id)
        })
        .await
        .unwrap_or_else(|| panic!("expected tool span for call_id={call_id} in Phoenix"));

        assert_eq!(
            attr_str(&span, "gen_ai.operation.name"),
            Some("execute_tool"),
            "tool span operation should be execute_tool"
        );
    }
}

#[tokio::test(flavor = "current_thread")]
async fn test_llmmetry_exports_streaming_success_span_to_phoenix_via_otlp() {
    let cfg = PhoenixConfig::from_env();
    ensure_phoenix_healthy(&cfg.base_url).await;

    let (base_url, _server) = start_sse_server(vec![
        "data: {\"choices\":[{\"delta\":{\"content\":\"hello from stream\"}}]}\n\n",
        "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n",
        "data: [DONE]\n\n",
    ])
    .await
    .unwrap_or_else(|e| panic!("failed to start local mock SSE LLM server for Phoenix e2e: {e}"));

    let (_guard, provider) = setup_otel_to_phoenix(&cfg.otlp_traces_endpoint, "phoenix-loop-e2e");
    let model_name = format!("phoenix-loop-stream-ok-{}", unique_suffix());

    let sink = InMemorySink::new();
    let plugin = Arc::new(
        LLMMetryPlugin::new(sink)
            .with_model(model_name.clone())
            .with_provider("test-provider"),
    ) as Arc<dyn AgentPlugin>;

    let client = genai::Client::builder()
        .with_service_target_resolver_fn(move |mut t: genai::ServiceTarget| {
            t.endpoint = genai::resolver::Endpoint::from_owned(base_url.clone());
            t.auth = genai::resolver::AuthData::from_single("test-key");
            Ok(t)
        })
        .build();

    let config = AgentConfig::new(model_name.clone()).with_plugin(plugin);
    let state = AgentState::with_initial_state("phoenix-stream-ok-state", json!({}))
        .with_message(Message::user("hi"));
    let tools: HashMap<String, Arc<dyn Tool>> = HashMap::new();

    let events: Vec<_> = run_loop_stream(client, config, state, tools, Default::default())
        .collect()
        .await;
    assert!(
        events
            .iter()
            .any(|e| matches!(e, AgentEvent::TextDelta { .. })),
        "should produce text deltas"
    );
    assert!(
        !events.iter().any(|e| matches!(e, AgentEvent::Error { .. })),
        "should not produce errors"
    );

    let _ = provider.force_flush();

    let span = wait_for_chat_span(&cfg.project_spans_url, &model_name)
        .await
        .expect("expected streaming success chat span exported to Phoenix");
    assert!(
        attr_str(&span, "error.type").is_none(),
        "success span should not have error.type"
    );
    assert!(
        span.get("attributes")
            .and_then(Value::as_object)
            .expect("span attributes")
            .keys()
            .any(|k| k.starts_with("gen_ai.")),
        "expected at least one gen_ai.* attribute"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn test_llmmetry_exports_tool_error_span_to_phoenix_via_otlp() {
    let cfg = PhoenixConfig::from_env();
    ensure_phoenix_healthy(&cfg.base_url).await;

    let (_guard, provider) =
        setup_otel_to_phoenix(&cfg.otlp_traces_endpoint, "phoenix-loop-e2e");
    let provider_name = format!("phoenix-tool-err-provider-{}", unique_suffix());

    let sink = InMemorySink::new();
    let plugin = Arc::new(LLMMetryPlugin::new(sink).with_provider(provider_name.clone()))
        as Arc<dyn AgentPlugin>;

    let thread = AgentState::with_initial_state("phoenix-tool-err-state", json!({}))
        .with_message(Message::user("hi"));
    let result = StreamResult {
        text: "tools".into(),
        tool_calls: vec![ToolCall::new("phoenix_err_call", "phoenix_bad", json!({}))],
        usage: None,
    };

    let mut tools: HashMap<String, Arc<dyn Tool>> = HashMap::new();
    tools.insert(
        "phoenix_bad".to_string(),
        Arc::new(FailingTool { id: "phoenix_bad" }),
    );

    let _ = execute_tools_with_plugins(thread, &result, &tools, true, &[plugin])
        .await
        .expect("tool execution with observability plugin should succeed even on tool error");

    let _ = provider.force_flush();

    let span = wait_for_span(&cfg.project_spans_url, |span| {
        span.get("name")
            .and_then(Value::as_str)
            .is_some_and(|name| name.starts_with("execute_tool "))
            && attr_str(span, "gen_ai.provider.name") == Some(provider_name.as_str())
            && attr_str(span, "gen_ai.tool.call.id") == Some("phoenix_err_call")
    })
    .await
    .expect("expected tool error span in Phoenix");

    assert_eq!(
        attr_str(&span, "error.type"),
        Some("tool_error"),
        "tool error span should have error.type = tool_error"
    );
}
