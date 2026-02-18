use async_trait::async_trait;
use axum::body::to_bytes;
use axum::http::{Request, StatusCode};
use carve_agent::contracts::plugin::AgentPlugin;
use carve_agent::contracts::runtime::phase::{Phase, StepContext};
use carve_agent::contracts::storage::AgentStateReader;
use carve_agent::orchestrator::{AgentDefinition, AgentOs, AgentOsBuilder};
use carve_agentos_server::http::{router, AppState};
use carve_thread_store_adapters::MemoryStore;
use serde_json::json;
use std::sync::Arc;
use tower::ServiceExt;

struct SkipInferencePlugin;

#[async_trait]
impl AgentPlugin for SkipInferencePlugin {
    fn id(&self) -> &str {
        "skip_inference_e2e_http_matrix"
    }

    async fn on_phase(
        &self,
        phase: Phase,
        step: &mut StepContext<'_>,
        _ctx: &carve_agent::prelude::AgentState,
    ) {
        if phase == Phase::BeforeInference {
            step.skip_inference = true;
        }
    }
}

fn make_os(store: Arc<MemoryStore>) -> AgentOs {
    let def = AgentDefinition {
        id: "test".to_string(),
        plugins: vec![Arc::new(SkipInferencePlugin)],
        ..Default::default()
    };

    AgentOsBuilder::new()
        .with_agent("test", def)
        .with_agent_state_store(store)
        .build()
        .expect("failed to build AgentOs")
}

async fn post_json(app: axum::Router, uri: &str, payload: serde_json::Value) -> (StatusCode, String) {
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(uri)
                .header("content-type", "application/json")
                .body(axum::body::Body::from(payload.to_string()))
                .expect("request build should succeed"),
        )
        .await
        .expect("app should handle request");
    let status = resp.status();
    let body = to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .expect("response body should be readable");
    let text = String::from_utf8(body.to_vec()).expect("response body must be utf-8");
    (status, text)
}

#[tokio::test]
async fn e2e_http_matrix_96() {
    let store = Arc::new(MemoryStore::new());
    let os = Arc::new(make_os(store.clone()));
    let app = router(AppState {
        os,
        read_store: store.clone(),
    });

    let content_cases = [
        "hello",
        "plain text",
        "line1\\nline2",
        "json-like {\"a\":1}",
        "symbols !@#$%^&*()",
        "1234567890",
        "keep it short",
        "context test",
        "state test",
        "final case",
        "extra case 01",
        "extra case 02",
        "extra case 03",
        "extra case 04",
        "extra case 05",
        "extra case 06",
    ];

    let run_modes = ["fixed", "alt", "auto_like"];

    let mut executed = 0usize;

    for (content_idx, content) in content_cases.iter().enumerate() {
        for (run_mode_idx, run_mode) in run_modes.iter().enumerate() {
            let thread_id = format!("matrix-ai-{content_idx}-{run_mode_idx}");
            let ai_run_id = match *run_mode {
                "fixed" => Some(format!("r-fixed-{content_idx}-{run_mode_idx}")),
                "alt" => Some(format!("r-alt-{content_idx}-{run_mode_idx}")),
                _ => None,
            };

            let ai_payload = json!({
                "sessionId": thread_id,
                "input": content,
                "runId": ai_run_id,
            });
            let (status, body) =
                post_json(app.clone(), "/v1/agents/test/runs/ai-sdk/sse", ai_payload).await;
            assert_eq!(status, StatusCode::OK);
            assert!(
                body.contains(r#""type":"start""#),
                "missing ai-sdk start event: {body}"
            );
            assert!(
                body.contains(r#""type":"finish""#),
                "missing ai-sdk finish event: {body}"
            );

            let ai_saved = store
                .load_agent_state(&thread_id)
                .await
                .expect("load should not fail")
                .expect("thread should be persisted");
            assert!(
                ai_saved.messages.iter().any(|m| m.content == *content),
                "persisted ai-sdk thread missing user content"
            );
            executed += 1;

            let ag_thread_id = format!("matrix-ag-{content_idx}-{run_mode_idx}");
            let ag_run_id = match *run_mode {
                "fixed" => format!("ag-fixed-{content_idx}-{run_mode_idx}"),
                "alt" => format!("ag-alt-{content_idx}-{run_mode_idx}"),
                _ => format!("ag-auto-like-{content_idx}-{run_mode_idx}"),
            };

            let ag_payload = json!({
                "threadId": ag_thread_id,
                "runId": ag_run_id,
                "messages": [{"role": "user", "content": content}],
                "tools": []
            });
            let (status, body) =
                post_json(app.clone(), "/v1/agents/test/runs/ag-ui/sse", ag_payload).await;
            assert_eq!(status, StatusCode::OK);
            assert!(
                body.contains(r#""type":"RUN_STARTED""#),
                "missing ag-ui RUN_STARTED event: {body}"
            );
            assert!(
                body.contains(r#""type":"RUN_FINISHED""#),
                "missing ag-ui RUN_FINISHED event: {body}"
            );

            let ag_saved = store
                .load_agent_state(&format!("matrix-ag-{content_idx}-{run_mode_idx}"))
                .await
                .expect("load should not fail")
                .expect("thread should be persisted");
            assert!(
                ag_saved.messages.iter().any(|m| m.content == *content),
                "persisted ag-ui thread missing user content"
            );
            executed += 1;
        }
    }

    assert_eq!(executed, 96, "e2e scenario count drifted");
}
