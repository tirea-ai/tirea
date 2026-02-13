//! End-to-end tests using TensorZero as LLM gateway and evaluation tool.
//!
//! These tests require:
//! 1. TensorZero + ClickHouse running via docker-compose
//! 2. `DEEPSEEK_API_KEY` set in the environment
//!
//! Run with:
//! ```bash
//! ./scripts/e2e-tensorzero.sh
//! ```
//!
//! Or manually:
//! ```bash
//! docker compose -f e2e/tensorzero/docker-compose.yml up -d --wait
//! DEEPSEEK_API_KEY=<key> cargo test --package carve-agentos-server --test e2e_tensorzero -- --ignored --nocapture
//! docker compose -f e2e/tensorzero/docker-compose.yml down -v
//! ```

use async_trait::async_trait;
use axum::body::to_bytes;
use axum::http::{Request, StatusCode};
use carve_agent::{
    AgentDefinition, AgentOsBuilder, MemoryStorage, ModelDefinition, Storage, Tool, ToolDescriptor,
    ToolError, ToolResult,
};
use carve_agentos_server::http::{router, AppState};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;
use tower::ServiceExt;

/// Trailing slash is required: genai's OpenAI adapter uses Url::join("chat/completions"),
/// which needs a trailing slash to resolve correctly.
const TENSORZERO_ENDPOINT: &str = "http://localhost:3000/openai/v1/";
const TENSORZERO_CHAT_URL: &str = "http://localhost:3000/openai/v1/chat/completions";
const TENSORZERO_FEEDBACK_URL: &str = "http://localhost:3000/feedback";

fn has_deepseek_key() -> bool {
    std::env::var("DEEPSEEK_API_KEY").is_ok()
}

fn tensorzero_reachable() -> bool {
    std::net::TcpStream::connect("127.0.0.1:3000").is_ok()
}

fn make_os() -> carve_agent::AgentOs {
    // Model name: "openai::tensorzero::function_name::agent_chat"
    //   - genai sees "openai::" prefix → selects OpenAI adapter (→ /v1/chat/completions)
    //   - genai strips the "openai::" namespace → sends "tensorzero::function_name::agent_chat"
    //     as the model in the request body, which is what TensorZero expects.
    // The ServiceTargetResolver overrides the endpoint to TensorZero's OpenAI-compat API.
    let tz_client = genai::Client::builder()
        .with_service_target_resolver_fn(|mut t: genai::ServiceTarget| {
            t.endpoint = genai::resolver::Endpoint::from_owned(TENSORZERO_ENDPOINT);
            t.auth = genai::resolver::AuthData::from_single("test");
            Ok(t)
        })
        .build();

    let def = AgentDefinition {
        id: "deepseek".to_string(),
        model: "deepseek".to_string(),
        system_prompt: "You are a helpful assistant. Keep answers very brief.".to_string(),
        max_rounds: 1,
        ..Default::default()
    };

    AgentOsBuilder::new()
        .with_provider("tz", tz_client)
        .with_model(
            "deepseek",
            ModelDefinition::new("tz", "openai::tensorzero::function_name::agent_chat"),
        )
        .with_agent("deepseek", def)
        .build()
        .expect("failed to build AgentOs with TensorZero")
}

fn skip_unless_ready() -> bool {
    if !has_deepseek_key() {
        eprintln!("DEEPSEEK_API_KEY not set, skipping");
        return true;
    }
    if !tensorzero_reachable() {
        eprintln!("TensorZero not reachable at :3000, skipping. Run: docker compose -f e2e/tensorzero/docker-compose.yml up -d --wait");
        return true;
    }
    false
}

// ============================================================================
// AI SDK SSE via TensorZero
// ============================================================================

#[tokio::test]
#[ignore]
async fn e2e_tensorzero_ai_sdk_sse() {
    if skip_unless_ready() {
        return;
    }

    let os = Arc::new(make_os());
    let storage: Arc<dyn Storage> = Arc::new(MemoryStorage::new());
    let app = router(AppState {
        os,
        storage: storage.clone(),
    });

    let payload = json!({
        "sessionId": "tz-sdk-1",
        "input": "What is 2+2? Reply with just the number.",
        "runId": "r1"
    });

    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/agents/deepseek/runs/ai-sdk/sse")
                .header("content-type", "application/json")
                .body(axum::body::Body::from(payload.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(
        resp.headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok()),
        Some("text/event-stream")
    );

    let body = to_bytes(resp.into_body(), 1024 * 1024).await.unwrap();
    let text = String::from_utf8(body.to_vec()).unwrap();

    println!("=== AI SDK SSE via TensorZero ===");
    println!("{text}");

    // Protocol correctness.
    assert!(text.contains(r#""type":"start""#), "missing start event");
    assert!(
        text.contains(r#""type":"text-start""#),
        "missing text-start"
    );
    assert!(
        text.contains(r#""type":"text-delta""#),
        "missing text-delta — LLM produced no text?"
    );
    assert!(text.contains(r#""type":"text-end""#), "missing text-end");
    assert!(text.contains(r#""type":"finish""#), "missing finish");

    // Output quality: extract text-delta values and verify the answer.
    let deltas: String = text
        .lines()
        .filter(|l| l.starts_with("data: "))
        .filter_map(|l| serde_json::from_str::<serde_json::Value>(&l[6..]).ok())
        .filter(|v| v.get("type").and_then(|t| t.as_str()) == Some("text-delta"))
        .filter_map(|v| v.get("delta").and_then(|t| t.as_str()).map(String::from))
        .collect();
    assert!(
        deltas.contains('4'),
        "LLM did not answer '4' for 2+2. Got: {deltas}"
    );

    // Session persistence.
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    let saved = storage.load("tz-sdk-1").await.unwrap();
    assert!(saved.is_some(), "session not persisted");
}

// ============================================================================
// AG-UI SSE via TensorZero
// ============================================================================

#[tokio::test]
#[ignore]
async fn e2e_tensorzero_ag_ui_sse() {
    if skip_unless_ready() {
        return;
    }

    let os = Arc::new(make_os());
    let storage: Arc<dyn Storage> = Arc::new(MemoryStorage::new());
    let app = router(AppState {
        os,
        storage: storage.clone(),
    });

    let payload = json!({
        "threadId": "tz-agui-1",
        "runId": "r2",
        "messages": [
            {"role": "user", "content": "What is 3+3? Reply with just the number."}
        ],
        "tools": []
    });

    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/agents/deepseek/runs/ag-ui/sse")
                .header("content-type", "application/json")
                .body(axum::body::Body::from(payload.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(
        resp.headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok()),
        Some("text/event-stream")
    );

    let body = to_bytes(resp.into_body(), 1024 * 1024).await.unwrap();
    let text = String::from_utf8(body.to_vec()).unwrap();

    println!("=== AG-UI SSE via TensorZero ===");
    println!("{text}");

    // Protocol correctness.
    assert!(
        text.contains(r#""type":"RUN_STARTED""#),
        "missing RUN_STARTED"
    );
    assert!(
        text.contains(r#""type":"RUN_FINISHED""#),
        "missing RUN_FINISHED"
    );
    assert!(
        text.contains(r#""type":"TEXT_MESSAGE_CONTENT""#),
        "missing TEXT_MESSAGE_CONTENT — LLM produced no text?"
    );

    // Output quality: extract text content and verify the answer.
    let content: String = text
        .lines()
        .filter(|l| l.starts_with("data: "))
        .filter_map(|l| serde_json::from_str::<serde_json::Value>(&l[6..]).ok())
        .filter(|v| v.get("type").and_then(|t| t.as_str()) == Some("TEXT_MESSAGE_CONTENT"))
        .filter_map(|v| v.get("delta").and_then(|d| d.as_str()).map(String::from))
        .collect();
    assert!(
        content.contains('6'),
        "LLM did not answer '6' for 3+3. Got: {content}"
    );

    // Session persistence.
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    let saved = storage.load("tz-agui-1").await.unwrap();
    assert!(saved.is_some(), "session not persisted");
}

// ============================================================================
// TensorZero feedback API integration
// ============================================================================

#[tokio::test]
#[ignore]
async fn e2e_tensorzero_feedback() {
    if skip_unless_ready() {
        return;
    }

    // First, make a direct inference call to TensorZero to get an inference_id.
    let http = reqwest::Client::new();
    let resp = http
        .post(TENSORZERO_CHAT_URL)
        .json(&json!({
            "model": "tensorzero::function_name::agent_chat",
            "messages": [
                {"role": "user", "content": "What is 5+5? Reply with just the number."}
            ]
        }))
        .send()
        .await
        .expect("TensorZero inference request failed");

    assert_eq!(
        resp.status(),
        reqwest::StatusCode::OK,
        "TensorZero returned non-200"
    );

    let body: serde_json::Value = resp.json().await.expect("invalid JSON response");
    println!("=== TensorZero Inference Response ===");
    println!("{}", serde_json::to_string_pretty(&body).unwrap());

    let inference_id = body["id"]
        .as_str()
        .expect("missing inference_id in TensorZero response");
    let content = body["choices"][0]["message"]["content"]
        .as_str()
        .unwrap_or("");

    println!("Inference ID: {inference_id}");
    println!("Content: {content}");

    // Verify output quality.
    let answer_correct = content.contains("10");

    // Submit feedback to TensorZero.
    let feedback_resp = http
        .post(TENSORZERO_FEEDBACK_URL)
        .json(&json!({
            "inference_id": inference_id,
            "metric_name": "answer_correct",
            "value": answer_correct
        }))
        .send()
        .await
        .expect("TensorZero feedback request failed");

    println!(
        "Feedback response: {} {}",
        feedback_resp.status(),
        feedback_resp
            .text()
            .await
            .unwrap_or_else(|_| "<no body>".to_string())
    );

    assert!(
        answer_correct,
        "LLM did not answer '10' for 5+5. Got: {content}"
    );
}

// ============================================================================
// Helpers
// ============================================================================

/// A deterministic calculator tool for E2E tests.
struct CalculatorTool;

#[async_trait]
impl Tool for CalculatorTool {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor::new(
            "calculator",
            "Calculator",
            "Perform basic arithmetic. Supports add, subtract, multiply, divide.",
        )
        .with_parameters(json!({
            "type": "object",
            "properties": {
                "operation": {
                    "type": "string",
                    "enum": ["add", "subtract", "multiply", "divide"],
                    "description": "The arithmetic operation to perform"
                },
                "a": { "type": "number", "description": "First operand" },
                "b": { "type": "number", "description": "Second operand" }
            },
            "required": ["operation", "a", "b"]
        }))
    }

    async fn execute(
        &self,
        args: Value,
        _ctx: &carve_agent::Context<'_>,
    ) -> Result<ToolResult, ToolError> {
        let op = args["operation"]
            .as_str()
            .ok_or_else(|| ToolError::InvalidArguments("missing operation".into()))?;
        let a = args["a"]
            .as_f64()
            .ok_or_else(|| ToolError::InvalidArguments("missing a".into()))?;
        let b = args["b"]
            .as_f64()
            .ok_or_else(|| ToolError::InvalidArguments("missing b".into()))?;

        let result = match op {
            "add" => a + b,
            "subtract" => a - b,
            "multiply" => a * b,
            "divide" if b != 0.0 => a / b,
            "divide" => return Err(ToolError::ExecutionFailed("division by zero".into())),
            _ => return Err(ToolError::InvalidArguments(format!("unknown op: {op}"))),
        };

        Ok(ToolResult::success(
            "calculator",
            json!({ "result": result }),
        ))
    }
}

fn make_tz_client() -> genai::Client {
    genai::Client::builder()
        .with_service_target_resolver_fn(|mut t: genai::ServiceTarget| {
            t.endpoint = genai::resolver::Endpoint::from_owned(TENSORZERO_ENDPOINT);
            t.auth = genai::resolver::AuthData::from_single("test");
            Ok(t)
        })
        .build()
}

fn make_tool_os() -> carve_agent::AgentOs {
    let def = AgentDefinition {
        id: "calc".to_string(),
        model: "deepseek".to_string(),
        system_prompt: "You are a calculator assistant.\n\
            Rules:\n\
            - You MUST use the `calculator` tool to perform arithmetic.\n\
            - After getting the tool result, reply with just the number.\n\
            - Never compute in your head — always call the tool."
            .to_string(),
        max_rounds: 3,
        ..Default::default()
    };

    let tools: HashMap<String, Arc<dyn Tool>> =
        HashMap::from([("calculator".to_string(), Arc::new(CalculatorTool) as _)]);

    AgentOsBuilder::new()
        .with_provider("tz", make_tz_client())
        .with_model(
            "deepseek",
            ModelDefinition::new("tz", "openai::tensorzero::function_name::agent_chat"),
        )
        .with_tools(tools)
        .with_agent("calc", def)
        .build()
        .expect("failed to build AgentOs with TensorZero + calculator")
}

/// Helper: send a POST and return (status, body_text).
async fn post_sse(app: axum::Router, uri: &str, payload: Value) -> (StatusCode, String) {
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(uri)
                .header("content-type", "application/json")
                .body(axum::body::Body::from(payload.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();

    let status = resp.status();
    let body = to_bytes(resp.into_body(), 1024 * 1024).await.unwrap();
    let text = String::from_utf8(body.to_vec()).unwrap();
    (status, text)
}

fn extract_ai_sdk_text(sse: &str) -> String {
    sse.lines()
        .filter(|l| l.starts_with("data: "))
        .filter_map(|l| serde_json::from_str::<Value>(&l[6..]).ok())
        .filter(|v| v.get("type").and_then(|t| t.as_str()) == Some("text-delta"))
        .filter_map(|v| v.get("delta").and_then(|t| t.as_str()).map(String::from))
        .collect()
}

fn extract_agui_text(sse: &str) -> String {
    sse.lines()
        .filter(|l| l.starts_with("data: "))
        .filter_map(|l| serde_json::from_str::<Value>(&l[6..]).ok())
        .filter(|v| v.get("type").and_then(|t| t.as_str()) == Some("TEXT_MESSAGE_CONTENT"))
        .filter_map(|v| v.get("delta").and_then(|d| d.as_str()).map(String::from))
        .collect()
}

// ============================================================================
// Tool-using agent via TensorZero (AI SDK)
// ============================================================================

#[tokio::test]
#[ignore]
async fn e2e_tensorzero_ai_sdk_tool_call() {
    if skip_unless_ready() {
        return;
    }

    let os = Arc::new(make_tool_os());
    let storage: Arc<dyn Storage> = Arc::new(MemoryStorage::new());
    let app = router(AppState {
        os,
        storage: storage.clone(),
    });

    let (status, text) = post_sse(
        app,
        "/v1/agents/calc/runs/ai-sdk/sse",
        json!({
            "sessionId": "tz-sdk-tool",
            "input": "Use the calculator tool to compute 25 * 4. Reply with just the number.",
            "runId": "r-tool-1"
        }),
    )
    .await;

    println!("=== AI SDK Tool Call via TensorZero ===\n{text}");

    assert_eq!(status, StatusCode::OK);
    assert!(
        text.contains(r#""type":"tool-input-start""#),
        "missing tool-input-start — LLM didn't invoke calculator"
    );
    assert!(
        text.contains(r#""type":"tool-output-available""#),
        "missing tool-output-available"
    );

    let answer = extract_ai_sdk_text(&text);
    println!("Extracted text: {answer}");
    assert!(
        answer.contains("100"),
        "LLM did not answer '100' for 25*4. Got: {answer}"
    );
}

// ============================================================================
// Tool-using agent via TensorZero (AG-UI)
// ============================================================================

#[tokio::test]
#[ignore]
async fn e2e_tensorzero_ag_ui_tool_call() {
    if skip_unless_ready() {
        return;
    }

    let os = Arc::new(make_tool_os());
    let storage: Arc<dyn Storage> = Arc::new(MemoryStorage::new());
    let app = router(AppState {
        os,
        storage: storage.clone(),
    });

    let (status, text) = post_sse(
        app,
        "/v1/agents/calc/runs/ag-ui/sse",
        json!({
            "threadId": "tz-agui-tool",
            "runId": "r-tool-2",
            "messages": [
                {"role": "user", "content": "Use the calculator tool to add 200 and 300. Reply with just the number."}
            ],
            "tools": []
        }),
    )
    .await;

    println!("=== AG-UI Tool Call via TensorZero ===\n{text}");

    assert_eq!(status, StatusCode::OK);
    assert!(
        text.contains(r#""type":"RUN_STARTED""#),
        "missing RUN_STARTED"
    );
    assert!(
        text.contains(r#""type":"RUN_FINISHED""#),
        "missing RUN_FINISHED"
    );
    assert!(
        text.contains(r#""type":"STEP_STARTED""#),
        "missing STEP_STARTED — agent loop should emit step boundaries"
    );
    assert!(
        text.contains(r#""type":"STEP_FINISHED""#),
        "missing STEP_FINISHED — agent loop should emit step boundaries"
    );
    assert!(
        text.contains(r#""type":"TOOL_CALL_START""#),
        "missing TOOL_CALL_START"
    );
    assert!(
        text.contains(r#""type":"TOOL_CALL_END""#),
        "missing TOOL_CALL_END"
    );

    let answer = extract_agui_text(&text);
    println!("Extracted text: {answer}");
    assert!(
        answer.contains("500"),
        "LLM did not answer '500' for 200+300. Got: {answer}"
    );
}

// ============================================================================
// Multi-turn conversation via TensorZero (AI SDK)
// ============================================================================

#[tokio::test]
#[ignore]
async fn e2e_tensorzero_ai_sdk_multiturn() {
    if skip_unless_ready() {
        return;
    }

    let os = Arc::new(make_os());
    let storage: Arc<dyn Storage> = Arc::new(MemoryStorage::new());

    // Turn 1.
    let app1 = router(AppState {
        os: os.clone(),
        storage: storage.clone(),
    });

    let (status, text1) = post_sse(
        app1,
        "/v1/agents/deepseek/runs/ai-sdk/sse",
        json!({
            "sessionId": "tz-sdk-multi",
            "input": "Remember the code word: banana. Just say OK.",
            "runId": "r-m1"
        }),
    )
    .await;

    println!("=== TZ Turn 1 ===\n{text1}");
    assert_eq!(status, StatusCode::OK);
    assert!(
        text1.contains(r#""type":"finish""#),
        "turn 1 did not finish"
    );

    tokio::time::sleep(std::time::Duration::from_millis(500)).await;

    // Turn 2.
    let app2 = router(AppState {
        os: os.clone(),
        storage: storage.clone(),
    });

    let (status, text2) = post_sse(
        app2,
        "/v1/agents/deepseek/runs/ai-sdk/sse",
        json!({
            "sessionId": "tz-sdk-multi",
            "input": "What was the code word? Reply with just the word.",
            "runId": "r-m2"
        }),
    )
    .await;

    println!("=== TZ Turn 2 ===\n{text2}");
    assert_eq!(status, StatusCode::OK);

    let answer = extract_ai_sdk_text(&text2);
    println!("Turn 2 text: {answer}");
    assert!(
        answer.to_lowercase().contains("banana"),
        "LLM did not recall 'banana'. Got: {answer}"
    );
}

// ============================================================================
// AI SDK: error event via max rounds exceeded (TensorZero)
// ============================================================================

#[tokio::test]
#[ignore]
async fn e2e_tensorzero_ai_sdk_error_max_rounds() {
    if skip_unless_ready() {
        return;
    }

    let def = AgentDefinition {
        id: "limited".to_string(),
        model: "deepseek".to_string(),
        system_prompt:
            "You MUST use the calculator tool for every question. Never answer directly."
                .to_string(),
        max_rounds: 1,
        ..Default::default()
    };

    let tools: HashMap<String, Arc<dyn Tool>> =
        HashMap::from([("calculator".to_string(), Arc::new(CalculatorTool) as _)]);

    let os = Arc::new(
        AgentOsBuilder::new()
            .with_provider("tz", make_tz_client())
            .with_model(
                "deepseek",
                ModelDefinition::new("tz", "openai::tensorzero::function_name::agent_chat"),
            )
            .with_tools(tools)
            .with_agent("limited", def)
            .build()
            .expect("failed to build limited AgentOs"),
    );

    let storage: Arc<dyn Storage> = Arc::new(MemoryStorage::new());
    let app = router(AppState {
        os,
        storage: storage.clone(),
    });

    let (status, text) = post_sse(
        app,
        "/v1/agents/limited/runs/ai-sdk/sse",
        json!({
            "sessionId": "tz-sdk-error",
            "input": "Use the calculator to add 1 and 2.",
            "runId": "r-err-sdk"
        }),
    )
    .await;

    println!("=== TZ AI SDK Error Response ===\n{text}");

    assert_eq!(status, StatusCode::OK);
    assert!(text.contains(r#""type":"start""#), "missing start event");

    assert!(
        text.contains(r#""type":"error""#),
        "missing error event — max rounds should trigger an error. Response:\n{text}"
    );
    assert!(
        text.contains("Max rounds") || text.contains("max rounds"),
        "error event should mention max rounds exceeded"
    );

    assert!(
        text.contains(r#""type":"tool-input-start""#),
        "missing tool-input-start — LLM should have called the tool before hitting max rounds"
    );
}

// ============================================================================
// AI SDK: Multi-step tool call (TensorZero)
// ============================================================================

#[tokio::test]
#[ignore]
async fn e2e_tensorzero_ai_sdk_multistep_tool() {
    if skip_unless_ready() {
        return;
    }

    let os = Arc::new(make_tool_os());
    let storage: Arc<dyn Storage> = Arc::new(MemoryStorage::new());
    let app = router(AppState {
        os,
        storage: storage.clone(),
    });

    let (status, text) = post_sse(
        app,
        "/v1/agents/calc/runs/ai-sdk/sse",
        json!({
            "sessionId": "tz-sdk-multistep",
            "input": "Use the calculator to multiply 12 by 5. Reply with just the number.",
            "runId": "r-ms-sdk"
        }),
    )
    .await;

    println!("=== TZ AI SDK Multi-Step Response ===\n{text}");

    assert_eq!(status, StatusCode::OK);
    assert!(text.contains(r#""type":"start""#), "missing start event");
    assert!(text.contains(r#""type":"finish""#), "missing finish event");

    let start_step_count = text.matches(r#""type":"start-step""#).count();
    let finish_step_count = text.matches(r#""type":"finish-step""#).count();

    println!("start-step count: {start_step_count}, finish-step count: {finish_step_count}");

    assert!(
        start_step_count >= 2,
        "expected >= 2 start-step events, got {start_step_count}"
    );
    assert!(
        finish_step_count >= 2,
        "expected >= 2 finish-step events, got {finish_step_count}"
    );

    assert!(
        text.contains(r#""type":"tool-input-start""#),
        "missing tool-input-start"
    );
    assert!(
        text.contains(r#""type":"tool-output-available""#),
        "missing tool-output-available"
    );

    let answer = extract_ai_sdk_text(&text);
    println!("Extracted text: {answer}");
    assert!(
        answer.contains("60"),
        "LLM did not answer '60' for 12*5. Got: {answer}"
    );

    let events: Vec<String> = text
        .lines()
        .filter(|l| l.starts_with("data: "))
        .filter_map(|l| serde_json::from_str::<Value>(&l[6..]).ok())
        .filter_map(|v| {
            let t = v.get("type")?.as_str()?;
            match t {
                "start" | "finish" | "start-step" | "finish-step" => Some(t.to_string()),
                _ => None,
            }
        })
        .collect();

    println!("Lifecycle event sequence: {events:?}");
    assert_eq!(events.first().map(|s| s.as_str()), Some("start"));
    assert_eq!(events.last().map(|s| s.as_str()), Some("finish"));
}

// ============================================================================
// Multi-turn conversation via TensorZero (AG-UI)
// ============================================================================

#[tokio::test]
#[ignore]
async fn e2e_tensorzero_ag_ui_multiturn() {
    if skip_unless_ready() {
        return;
    }

    let os = Arc::new(make_os());
    let storage: Arc<dyn Storage> = Arc::new(MemoryStorage::new());

    // Turn 1.
    let app1 = router(AppState {
        os: os.clone(),
        storage: storage.clone(),
    });

    let (status, _) = post_sse(
        app1,
        "/v1/agents/deepseek/runs/ag-ui/sse",
        json!({
            "threadId": "tz-agui-multi",
            "runId": "r-am1",
            "messages": [
                {"role": "user", "content": "Remember the color: purple. Just say OK."}
            ],
            "tools": []
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    tokio::time::sleep(std::time::Duration::from_millis(500)).await;

    // Turn 2: AG-UI sends full history.
    let app2 = router(AppState {
        os: os.clone(),
        storage: storage.clone(),
    });

    let (status, text2) = post_sse(
        app2,
        "/v1/agents/deepseek/runs/ag-ui/sse",
        json!({
            "threadId": "tz-agui-multi",
            "runId": "r-am2",
            "messages": [
                {"role": "user", "content": "Remember the color: purple. Just say OK."},
                {"role": "assistant", "content": "OK"},
                {"role": "user", "content": "What color did I mention? Reply with just the color."}
            ],
            "tools": []
        }),
    )
    .await;

    println!("=== TZ AG-UI Turn 2 ===\n{text2}");
    assert_eq!(status, StatusCode::OK);

    let answer = extract_agui_text(&text2);
    println!("Turn 2 text: {answer}");
    assert!(
        answer.to_lowercase().contains("purple"),
        "LLM did not recall 'purple'. Got: {answer}"
    );
}

// ============================================================================
// AG-UI: RUN_ERROR via max rounds exceeded (TensorZero)
// ============================================================================

#[tokio::test]
#[ignore]
async fn e2e_tensorzero_ag_ui_run_error_max_rounds() {
    if skip_unless_ready() {
        return;
    }

    // max_rounds=1 + tool = the agent will call the tool, then need a second round → error.
    let def = AgentDefinition {
        id: "limited".to_string(),
        model: "deepseek".to_string(),
        system_prompt:
            "You MUST use the calculator tool for every question. Never answer directly."
                .to_string(),
        max_rounds: 1,
        ..Default::default()
    };

    let tools: HashMap<String, Arc<dyn Tool>> =
        HashMap::from([("calculator".to_string(), Arc::new(CalculatorTool) as _)]);

    let os = Arc::new(
        AgentOsBuilder::new()
            .with_provider("tz", make_tz_client())
            .with_model(
                "deepseek",
                ModelDefinition::new("tz", "openai::tensorzero::function_name::agent_chat"),
            )
            .with_tools(tools)
            .with_agent("limited", def)
            .build()
            .expect("failed to build limited AgentOs"),
    );

    let storage: Arc<dyn Storage> = Arc::new(MemoryStorage::new());
    let app = router(AppState {
        os,
        storage: storage.clone(),
    });

    let (status, text) = post_sse(
        app,
        "/v1/agents/limited/runs/ag-ui/sse",
        json!({
            "threadId": "tz-agui-error",
            "runId": "r-err-1",
            "messages": [
                {"role": "user", "content": "Use the calculator to add 1 and 2."}
            ],
            "tools": []
        }),
    )
    .await;

    println!("=== TZ AG-UI RUN_ERROR Response ===\n{text}");

    assert_eq!(status, StatusCode::OK);
    assert!(
        text.contains(r#""type":"RUN_STARTED""#),
        "missing RUN_STARTED"
    );

    assert!(
        text.contains(r#""type":"RUN_ERROR""#),
        "missing RUN_ERROR — max rounds should trigger an error. Response:\n{text}"
    );
    assert!(
        text.contains("Max rounds") || text.contains("max rounds"),
        "RUN_ERROR should mention max rounds exceeded"
    );
}

// ============================================================================
// AG-UI: Multi-step tool call (verify multiple STEP cycles, TensorZero)
// ============================================================================

#[tokio::test]
#[ignore]
async fn e2e_tensorzero_ag_ui_multistep_tool() {
    if skip_unless_ready() {
        return;
    }

    let os = Arc::new(make_tool_os());
    let storage: Arc<dyn Storage> = Arc::new(MemoryStorage::new());
    let app = router(AppState {
        os,
        storage: storage.clone(),
    });

    let (status, text) = post_sse(
        app,
        "/v1/agents/calc/runs/ag-ui/sse",
        json!({
            "threadId": "tz-agui-multistep",
            "runId": "r-ms-1",
            "messages": [
                {"role": "user", "content": "Use the calculator to multiply 9 by 6. Reply with just the number."}
            ],
            "tools": []
        }),
    )
    .await;

    println!("=== TZ AG-UI Multi-Step Response ===\n{text}");

    assert_eq!(status, StatusCode::OK);
    assert!(
        text.contains(r#""type":"RUN_STARTED""#),
        "missing RUN_STARTED"
    );
    assert!(
        text.contains(r#""type":"RUN_FINISHED""#),
        "missing RUN_FINISHED"
    );

    // Should have >= 2 STEP cycles (tool call round + text round).
    let step_started_count = text.matches(r#""type":"STEP_STARTED""#).count();
    let step_finished_count = text.matches(r#""type":"STEP_FINISHED""#).count();

    println!(
        "STEP_STARTED count: {step_started_count}, STEP_FINISHED count: {step_finished_count}"
    );

    assert!(
        step_started_count >= 2,
        "expected >= 2 STEP_STARTED events, got {step_started_count}"
    );
    assert!(
        step_finished_count >= 2,
        "expected >= 2 STEP_FINISHED events, got {step_finished_count}"
    );

    // Tool events.
    assert!(
        text.contains(r#""type":"TOOL_CALL_START""#),
        "missing TOOL_CALL_START"
    );
    assert!(
        text.contains(r#""type":"TOOL_CALL_RESULT""#),
        "missing TOOL_CALL_RESULT"
    );

    // Final answer.
    let answer = extract_agui_text(&text);
    println!("Extracted text: {answer}");
    assert!(
        answer.contains("54"),
        "LLM did not answer '54' for 9*6. Got: {answer}"
    );

    // Event ordering: RUN_STARTED first, RUN_FINISHED last.
    let events: Vec<String> = text
        .lines()
        .filter(|l| l.starts_with("data: "))
        .filter_map(|l| serde_json::from_str::<Value>(&l[6..]).ok())
        .filter_map(|v| {
            let t = v.get("type")?.as_str()?;
            match t {
                "RUN_STARTED" | "RUN_FINISHED" | "STEP_STARTED" | "STEP_FINISHED" => {
                    Some(t.to_string())
                }
                _ => None,
            }
        })
        .collect();

    println!("Lifecycle event sequence: {events:?}");
    assert_eq!(events.first().map(|s| s.as_str()), Some("RUN_STARTED"));
    assert_eq!(events.last().map(|s| s.as_str()), Some("RUN_FINISHED"));
}
