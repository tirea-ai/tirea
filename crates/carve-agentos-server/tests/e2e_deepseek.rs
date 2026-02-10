//! End-to-end tests for AI SDK and AG-UI SSE endpoints using DeepSeek.
//!
//! These tests require a valid `DEEPSEEK_API_KEY` environment variable.
//! Run with:
//! ```bash
//! DEEPSEEK_API_KEY=<key> cargo test --package carve-agentos-server --test e2e_deepseek -- --ignored --nocapture
//! ```

use axum::body::to_bytes;
use axum::http::{Request, StatusCode};
use carve_agent::{AgentDefinition, AgentOsBuilder, MemoryStorage, Storage};
use carve_agentos_server::http::{router, AppState};
use serde_json::json;
use std::sync::Arc;
use tower::ServiceExt;

fn has_deepseek_key() -> bool {
    std::env::var("DEEPSEEK_API_KEY").is_ok()
}

fn make_os() -> carve_agent::AgentOs {
    let def = AgentDefinition {
        id: "deepseek".to_string(),
        model: "deepseek-chat".to_string(),
        system_prompt: "You are a helpful assistant. Keep answers very brief.".to_string(),
        max_rounds: 1,
        ..Default::default()
    };

    AgentOsBuilder::new()
        .with_agent("deepseek", def)
        .build()
        .expect("failed to build AgentOs")
}

#[tokio::test]
#[ignore]
async fn e2e_ai_sdk_sse_with_deepseek() {
    if !has_deepseek_key() {
        eprintln!("DEEPSEEK_API_KEY not set, skipping");
        return;
    }

    let os = Arc::new(make_os());
    let storage: Arc<dyn Storage> = Arc::new(MemoryStorage::new());
    let app = router(AppState {
        os,
        storage: storage.clone(),
    });

    let payload = json!({
        "sessionId": "e2e-sdk",
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

    println!("=== AI SDK SSE Response ===");
    println!("{text}");

    assert!(
        text.contains(r#""type":"start""#),
        "missing start event"
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

    // Verify run-info event carries correct session/run IDs.
    assert!(
        text.contains(r#""threadId":"e2e-sdk""#),
        "missing threadId in run-info"
    );
    assert!(
        text.contains(r#""runId":"r1""#),
        "missing runId in run-info"
    );

    // Wait briefly for the checkpoint spawn to flush.
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;

    // Session should be persisted with the user message.
    let saved = storage.load("e2e-sdk").await.unwrap();
    assert!(saved.is_some(), "session not persisted");
    let saved = saved.unwrap();
    assert!(
        saved.messages.iter().any(|m| m.content.contains("2+2")),
        "user message not found in persisted session"
    );
}

#[tokio::test]
#[ignore]
async fn e2e_ag_ui_sse_with_deepseek() {
    if !has_deepseek_key() {
        eprintln!("DEEPSEEK_API_KEY not set, skipping");
        return;
    }

    let os = Arc::new(make_os());
    let storage: Arc<dyn Storage> = Arc::new(MemoryStorage::new());
    let app = router(AppState {
        os,
        storage: storage.clone(),
    });

    let payload = json!({
        "threadId": "e2e-agui",
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

    println!("=== AG-UI SSE Response ===");
    println!("{text}");

    assert!(
        text.contains(r#""type":"RUN_STARTED""#),
        "missing RUN_STARTED"
    );
    assert!(
        text.contains(r#""type":"RUN_FINISHED""#),
        "missing RUN_FINISHED"
    );
    // Should have some text content from the LLM.
    assert!(
        text.contains(r#""type":"TEXT_MESSAGE_CONTENT""#),
        "missing TEXT_MESSAGE_CONTENT — LLM produced no text?"
    );

    // Wait briefly for the checkpoint spawn to flush.
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;

    // Session should be persisted.
    let saved = storage.load("e2e-agui").await.unwrap();
    assert!(saved.is_some(), "session not persisted");
    let saved = saved.unwrap();
    assert!(
        saved
            .messages
            .iter()
            .any(|m| m.content.contains("3+3") || m.content.contains("3 + 3")),
        "user message not found in persisted session"
    );
}
