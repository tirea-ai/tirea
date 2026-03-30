//! MCP protocol end-to-end tests.
//!
//! Verifies the full flow: MCP client → JSON-RPC → McpServer → AgentMcpTool →
//! AgentRuntime → event collection → tool result response.

use async_trait::async_trait;
use awaken_contract::contract::content::ContentBlock;
use awaken_contract::contract::executor::{InferenceExecutionError, InferenceRequest};
use awaken_contract::contract::inference::{StopReason, StreamResult, TokenUsage};
use awaken_contract::registry_spec::AgentSpec;
use awaken_runtime::builder::AgentRuntimeBuilder;
use awaken_runtime::registry::traits::ModelEntry;
use awaken_server::app::{AppState, ServerConfig};
use awaken_server::routes::build_router;
use awaken_stores::memory::InMemoryStore;
use axum::body::to_bytes;
use axum::http::{Request, StatusCode};
use serde_json::{Value, json};
use std::sync::Arc;
use tower::ServiceExt;

// ── Mock executor that returns a fixed text response ──

struct EchoExecutor;

#[async_trait]
impl awaken_contract::contract::executor::LlmExecutor for EchoExecutor {
    async fn execute(
        &self,
        request: InferenceRequest,
    ) -> Result<StreamResult, InferenceExecutionError> {
        // Echo back the last user message as assistant response.
        let user_text = request
            .messages
            .iter()
            .rev()
            .find_map(|m| {
                if m.role == awaken_contract::contract::message::Role::User {
                    Some(m.text())
                } else {
                    None
                }
            })
            .unwrap_or_default();

        Ok(StreamResult {
            content: vec![ContentBlock::text(format!("echo: {user_text}"))],
            tool_calls: vec![],
            usage: Some(TokenUsage::default()),
            stop_reason: Some(StopReason::EndTurn),
            has_incomplete_tool_calls: false,
        })
    }

    fn name(&self) -> &str {
        "echo"
    }
}

// ── App setup ──

fn make_mcp_app() -> axum::Router {
    let runtime = {
        let builder = AgentRuntimeBuilder::new()
            .with_model(
                "test-model",
                ModelEntry {
                    provider: "mock".into(),
                    model_name: "mock-model".into(),
                },
            )
            .with_provider("mock", Arc::new(EchoExecutor))
            .with_agent_spec(AgentSpec {
                id: "echo".into(),
                model: "test-model".into(),
                system_prompt: "You are an echo bot".into(),
                max_rounds: 2,
                ..Default::default()
            });
        Arc::new(builder.build().expect("build runtime"))
    };
    let store = Arc::new(InMemoryStore::new());
    let mailbox_store = Arc::new(awaken_stores::InMemoryMailboxStore::new());
    let mailbox = Arc::new(awaken_server::mailbox::Mailbox::new(
        runtime.clone(),
        mailbox_store,
        "test".to_string(),
        awaken_server::mailbox::MailboxConfig::default(),
    ));
    let state = AppState::new(
        runtime.clone(),
        mailbox,
        store,
        runtime.resolver_arc(),
        ServerConfig::default(),
    );
    build_router().with_state(state)
}

async fn mcp_post(app: axum::Router, payload: Value) -> (StatusCode, Value) {
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/mcp")
                .header("content-type", "application/json")
                .body(axum::body::Body::from(
                    serde_json::to_vec(&payload).unwrap(),
                ))
                .expect("request build"),
        )
        .await
        .expect("app should handle request");
    let status = resp.status();
    let body = to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .expect("body readable");
    let json: Value = serde_json::from_slice(&body).unwrap_or(json!(null));
    (status, json)
}

// ============================================================================
// Tests
// ============================================================================

#[tokio::test]
async fn mcp_initialize_returns_server_info() {
    let app = make_mcp_app();
    let (status, json) = mcp_post(
        app,
        json!({
            "jsonrpc": "2.0",
            "method": "initialize",
            "id": 1
        }),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert!(json["result"]["protocolVersion"].is_string());
    assert_eq!(json["result"]["serverInfo"]["name"], "awaken-mcp");
}

#[tokio::test]
async fn mcp_tools_list_discovers_agents() {
    let app = make_mcp_app();
    let (status, json) = mcp_post(
        app,
        json!({
            "jsonrpc": "2.0",
            "method": "tools/list",
            "id": 2
        }),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    let tools = json["result"]["tools"].as_array().unwrap();
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0]["name"], "echo");
    assert!(
        tools[0]["description"]
            .as_str()
            .unwrap()
            .contains("echo bot")
    );

    // Verify schema
    let schema = &tools[0]["inputSchema"];
    assert_eq!(schema["type"], "object");
    assert!(schema["properties"]["message"].is_object());
    let required = schema["required"].as_array().unwrap();
    assert!(required.contains(&json!("message")));
}

#[tokio::test]
async fn mcp_tools_call_runs_agent_and_returns_text() {
    let app = make_mcp_app();
    let (status, json) = mcp_post(
        app,
        json!({
            "jsonrpc": "2.0",
            "method": "tools/call",
            "params": {
                "name": "echo",
                "arguments": {
                    "message": "hello world"
                }
            },
            "id": 3
        }),
    )
    .await;

    assert_eq!(status, StatusCode::OK);

    // The response should contain the echoed text.
    let content = &json["result"]["content"];
    assert!(content.is_array(), "expected content array, got: {json}");
    let content_arr = content.as_array().unwrap();
    assert!(!content_arr.is_empty(), "content should not be empty");

    let text = content_arr[0]["text"].as_str().unwrap_or("");
    assert!(
        text.contains("echo: hello world"),
        "expected echo response, got: {text}"
    );

    // isError should be false.
    assert_eq!(json["result"]["isError"], false);
}

#[tokio::test]
async fn mcp_tools_call_unknown_tool_returns_error() {
    let app = make_mcp_app();
    let (status, json) = mcp_post(
        app,
        json!({
            "jsonrpc": "2.0",
            "method": "tools/call",
            "params": {
                "name": "nonexistent",
                "arguments": {
                    "message": "hello"
                }
            },
            "id": 4
        }),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    // MCP returns isError=true for tool errors, not HTTP errors.
    assert_eq!(json["result"]["isError"], true);
}

#[tokio::test]
async fn mcp_tools_call_missing_message_returns_tool_error() {
    let app = make_mcp_app();
    let (status, json) = mcp_post(
        app,
        json!({
            "jsonrpc": "2.0",
            "method": "tools/call",
            "params": {
                "name": "echo",
                "arguments": {}
            },
            "id": 5
        }),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["result"]["isError"], true);
    let text = json["result"]["content"][0]["text"].as_str().unwrap_or("");
    assert!(
        text.contains("message"),
        "error should mention 'message' param"
    );
}

#[tokio::test]
async fn mcp_ping_responds() {
    let app = make_mcp_app();
    let (status, json) = mcp_post(
        app,
        json!({
            "jsonrpc": "2.0",
            "method": "ping",
            "id": 6
        }),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert!(json["result"].is_object());
}

#[tokio::test]
async fn mcp_unknown_method_returns_error() {
    let app = make_mcp_app();
    let (status, json) = mcp_post(
        app,
        json!({
            "jsonrpc": "2.0",
            "method": "unknown/method",
            "id": 7
        }),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert!(json["error"].is_object());
    assert_eq!(json["error"]["code"], -32601);
}

// ── Stdio E2E ──

#[tokio::test]
async fn stdio_e2e_full_flow() {
    let runtime = {
        let builder = AgentRuntimeBuilder::new()
            .with_model(
                "test-model",
                ModelEntry {
                    provider: "mock".into(),
                    model_name: "mock-model".into(),
                },
            )
            .with_provider("mock", Arc::new(EchoExecutor))
            .with_agent_spec(AgentSpec {
                id: "echo".into(),
                model: "test-model".into(),
                system_prompt: "You are an echo bot".into(),
                max_rounds: 2,
                ..Default::default()
            });
        Arc::new(builder.build().expect("build runtime"))
    };

    let input = concat!(
        "{\"jsonrpc\":\"2.0\",\"method\":\"initialize\",\"id\":1}\n",
        "{\"jsonrpc\":\"2.0\",\"method\":\"notifications/initialized\"}\n",
        "{\"jsonrpc\":\"2.0\",\"method\":\"tools/list\",\"id\":2}\n",
        "{\"jsonrpc\":\"2.0\",\"method\":\"tools/call\",\"params\":{\"name\":\"echo\",\"arguments\":{\"message\":\"hi\"}},\"id\":3}\n",
    );

    let mut output = Vec::new();
    awaken_server::protocols::mcp::stdio::serve_stdio_io(runtime, input.as_bytes(), &mut output)
        .await;

    let output_str = String::from_utf8(output).unwrap();
    let lines: Vec<Value> = output_str
        .trim()
        .lines()
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect();

    // Should have responses for initialize(1), tools/list(2), tools/call(3).
    // Plus possibly progress/log notifications.
    let responses: Vec<&Value> = lines.iter().filter(|v| v.get("id").is_some()).collect();
    assert!(
        responses.len() >= 3,
        "expected at least 3 responses, got {}: {lines:?}",
        responses.len()
    );

    // Verify initialize response.
    let init = responses.iter().find(|v| v["id"] == 1).unwrap();
    assert!(init["result"]["protocolVersion"].is_string());

    // Verify tools/list response.
    let list = responses.iter().find(|v| v["id"] == 2).unwrap();
    let tools = list["result"]["tools"].as_array().unwrap();
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0]["name"], "echo");

    // Verify tools/call response — should contain echo text.
    let call = responses.iter().find(|v| v["id"] == 3).unwrap();
    let content = call["result"]["content"].as_array().unwrap();
    let text = content[0]["text"].as_str().unwrap_or("");
    assert!(
        text.contains("echo: hi"),
        "expected echo response, got: {text}"
    );

    // Check for progress/log notifications.
    let notifications: Vec<&Value> = lines
        .iter()
        .filter(|v| v.get("method").is_some() && v.get("id").is_none())
        .collect();
    // Should have at least one progress notification from the tool call.
    let has_progress = notifications
        .iter()
        .any(|n| n["method"] == "notifications/progress");
    assert!(
        has_progress,
        "expected progress notifications, got: {notifications:?}"
    );
}
