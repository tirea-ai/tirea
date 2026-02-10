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

use axum::body::to_bytes;
use axum::http::{Request, StatusCode};
use carve_agent::{AgentDefinition, AgentOsBuilder, MemoryStorage, ModelDefinition, Storage};
use carve_agentos_server::http::{router, AppState};
use serde_json::json;
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
    assert!(
        text.contains(r#""type":"message-start""#),
        "missing message-start"
    );
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
