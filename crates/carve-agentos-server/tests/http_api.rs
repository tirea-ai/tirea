use async_trait::async_trait;
use axum::body::to_bytes;
use axum::http::{Request, StatusCode};
use carve_agent::phase::Phase;
use carve_agent::plugin::AgentPlugin;
use carve_agent::{
    AgentDefinition, AgentOs, AgentOsBuilder, MemoryStorage, Session, StepContext, Storage,
};
use carve_agentos_server::http::{router, AppState};
use serde_json::json;
use std::sync::Arc;
use tower::ServiceExt;

struct SkipInferencePlugin;

#[async_trait]
impl AgentPlugin for SkipInferencePlugin {
    fn id(&self) -> &str {
        "skip_inference_test"
    }

    async fn on_phase(&self, phase: Phase, step: &mut StepContext<'_>) {
        if phase == Phase::BeforeInference {
            step.skip_inference = true;
        }
    }
}

fn make_os() -> AgentOs {
    let mut def = AgentDefinition::default();
    def.id = "test".to_string();
    def.plugins = vec![Arc::new(SkipInferencePlugin)];

    AgentOsBuilder::new()
        .with_agent("test", def)
        .build()
        .unwrap()
}

#[tokio::test]
async fn test_sessions_query_endpoints() {
    let os = Arc::new(make_os());
    let storage = Arc::new(MemoryStorage::new());

    let session = Session::new("s1").with_message(carve_agent::Message::user("hello"));
    storage.save(&session).await.unwrap();

    let app = router(AppState { os, storage });

    let resp = app
        .clone()
        .oneshot(Request::builder().uri("/v1/sessions").body(axum::body::Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let ids: Vec<String> = serde_json::from_slice(&body).unwrap();
    assert_eq!(ids, vec!["s1".to_string()]);

    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/v1/sessions/s1/messages")
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let msgs: Vec<carve_agent::Message> = serde_json::from_slice(&body).unwrap();
    assert_eq!(msgs.len(), 1);
    assert_eq!(msgs[0].content, "hello");
}

#[tokio::test]
async fn test_ai_sdk_sse_and_persists_session() {
    let os = Arc::new(make_os());
    let storage = Arc::new(MemoryStorage::new());
    let app = router(AppState {
        os: os.clone(),
        storage: storage.clone(),
    });

    let payload = json!({
        "sessionId": "t1",
        "input": "hi",
        "runId": "run_1"
    });

    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/agents/test/runs/ai-sdk/sse")
                .header("content-type", "application/json")
                .body(axum::body::Body::from(payload.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let text = String::from_utf8(body.to_vec()).unwrap();
    assert!(text.contains(r#""type":"message-start""#), "missing message-start: {text}");
    assert!(text.contains(r#""type":"text-start""#), "missing text-start: {text}");
    assert!(text.contains(r#""type":"text-end""#), "missing text-end: {text}");
    assert!(text.contains(r#""type":"finish""#), "missing finish: {text}");

    let saved = storage.load("t1").await.unwrap().unwrap();
    assert_eq!(saved.id, "t1");
    assert_eq!(saved.messages.len(), 1);
    assert_eq!(saved.messages[0].content, "hi");
}

#[tokio::test]
async fn test_agui_sse_and_persists_session() {
    let os = Arc::new(make_os());
    let storage = Arc::new(MemoryStorage::new());
    let app = router(AppState {
        os: os.clone(),
        storage: storage.clone(),
    });

    let payload = json!({
        "threadId": "th1",
        "runId": "r1",
        "messages": [
            {"role": "user", "content": "hello from agui"}
        ],
        "tools": []
    });

    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/agents/test/runs/ag-ui/sse")
                .header("content-type", "application/json")
                .body(axum::body::Body::from(payload.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let text = String::from_utf8(body.to_vec()).unwrap();
    assert!(
        text.contains(r#""type":"RUN_STARTED""#),
        "missing RUN_STARTED: {text}"
    );
    assert!(
        text.contains(r#""type":"RUN_FINISHED""#),
        "missing RUN_FINISHED: {text}"
    );

    let saved = storage.load("th1").await.unwrap().unwrap();
    assert_eq!(saved.id, "th1");
    assert_eq!(saved.messages.len(), 1);
}
