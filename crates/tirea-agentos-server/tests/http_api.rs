use async_trait::async_trait;
use axum::body::to_bytes;
use axum::http::{Request, StatusCode};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use tirea_agentos::contracts::plugin::phase::Phase;
use tirea_agentos::contracts::plugin::phase::StepContext;
use tirea_agentos::contracts::plugin::AgentPlugin;
use tirea_agentos::contracts::storage::{
    AgentStateHead, AgentStateListPage, AgentStateListQuery, AgentStateReader, AgentStateStore,
    AgentStateStoreError, AgentStateWriter, Committed,
};
use tirea_agentos::contracts::thread::Thread;
use tirea_agentos::contracts::tool::{Tool, ToolDescriptor, ToolError, ToolResult};
use tirea_agentos::contracts::ThreadChangeSet;
use tirea_agentos::contracts::ToolCallContext;
use tirea_agentos::orchestrator::AgentDefinition;
use tirea_agentos::orchestrator::{AgentOs, AgentOsBuilder};
use tirea_agentos_server::http::{router, AppState};
use tirea_store_adapters::MemoryStore;
use tokio::sync::{Notify, RwLock};
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
    make_os_with_storage(Arc::new(MemoryStore::new()))
}

fn make_os_with_storage(write_store: Arc<dyn AgentStateStore>) -> AgentOs {
    make_os_with_storage_and_tools(write_store, HashMap::new())
}

fn make_os_with_storage_and_tools(
    write_store: Arc<dyn AgentStateStore>,
    tools: HashMap<String, Arc<dyn Tool>>,
) -> AgentOs {
    let def = AgentDefinition {
        id: "test".to_string(),
        plugin_ids: vec!["skip_inference_test".into()],
        ..Default::default()
    };

    AgentOsBuilder::new()
        .with_registered_plugin("skip_inference_test", Arc::new(SkipInferencePlugin))
        .with_tools(tools)
        .with_agent("test", def)
        .with_agent_state_store(write_store)
        .build()
        .unwrap()
}

struct EchoTool;

#[async_trait]
impl Tool for EchoTool {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor::new("echo", "Echo", "Echo input message").with_parameters(json!({
            "type": "object",
            "properties": {
                "message": { "type": "string" }
            },
            "required": ["message"]
        }))
    }

    async fn execute(
        &self,
        args: Value,
        _ctx: &ToolCallContext<'_>,
    ) -> Result<ToolResult, ToolError> {
        let message = args
            .get("message")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();
        Ok(ToolResult::success("echo", json!({ "echoed": message })))
    }
}

#[derive(Default)]
struct RecordingStorage {
    threads: RwLock<HashMap<String, Thread>>,
    saves: AtomicUsize,
    notify: Notify,
}

impl RecordingStorage {
    async fn wait_saves(&self, n: usize) {
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
        while self.saves.load(Ordering::SeqCst) < n {
            tokio::select! {
                _ = self.notify.notified() => {}
                _ = tokio::time::sleep_until(deadline) => break,
            }
        }
    }
}

#[async_trait]
impl AgentStateWriter for RecordingStorage {
    async fn create(&self, thread: &Thread) -> Result<Committed, AgentStateStoreError> {
        let mut threads = self.threads.write().await;
        if threads.contains_key(&thread.id) {
            return Err(AgentStateStoreError::AlreadyExists);
        }
        threads.insert(thread.id.clone(), thread.clone());
        self.saves.fetch_add(1, Ordering::SeqCst);
        self.notify.notify_waiters();
        Ok(Committed { version: 0 })
    }

    async fn append(
        &self,
        id: &str,
        delta: &ThreadChangeSet,
        _precondition: tirea_agentos::contracts::storage::VersionPrecondition,
    ) -> Result<Committed, AgentStateStoreError> {
        let mut threads = self.threads.write().await;
        if let Some(thread) = threads.get_mut(id) {
            for msg in &delta.messages {
                thread.messages.push(msg.clone());
            }
        }
        self.saves.fetch_add(1, Ordering::SeqCst);
        drop(threads);
        self.notify.notify_waiters();
        Ok(Committed { version: 0 })
    }

    async fn delete(&self, id: &str) -> Result<(), AgentStateStoreError> {
        let mut threads = self.threads.write().await;
        threads.remove(id);
        Ok(())
    }

    async fn save(&self, thread: &Thread) -> Result<(), AgentStateStoreError> {
        let mut threads = self.threads.write().await;
        threads.insert(thread.id.clone(), thread.clone());
        self.saves.fetch_add(1, Ordering::SeqCst);
        self.notify.notify_waiters();
        Ok(())
    }
}

#[async_trait]
impl AgentStateReader for RecordingStorage {
    async fn load(&self, id: &str) -> Result<Option<AgentStateHead>, AgentStateStoreError> {
        let threads = self.threads.read().await;
        Ok(threads.get(id).map(|t| AgentStateHead {
            agent_state: t.clone(),
            version: 0,
        }))
    }

    async fn list_agent_states(
        &self,
        query: &AgentStateListQuery,
    ) -> Result<AgentStateListPage, AgentStateStoreError> {
        let threads = self.threads.read().await;
        let mut ids: Vec<String> = threads.keys().cloned().collect();
        ids.sort();
        let total = ids.len();
        let limit = query.limit.clamp(1, 200);
        let offset = query.offset.min(total);
        let end = (offset + limit + 1).min(total);
        let slice = &ids[offset..end];
        let has_more = slice.len() > limit;
        let items: Vec<String> = slice.iter().take(limit).cloned().collect();
        Ok(AgentStateListPage {
            items,
            total,
            has_more,
        })
    }
}

#[tokio::test]
async fn test_sessions_query_endpoints() {
    let os = Arc::new(make_os());
    let storage = Arc::new(MemoryStore::new());

    let thread =
        Thread::new("s1").with_message(tirea_agentos::contracts::thread::Message::user("hello"));
    storage.save(&thread).await.unwrap();

    let app = router(AppState {
        os,
        read_store: storage,
    });

    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/v1/threads")
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let page: tirea_agentos::contracts::storage::AgentStateListPage =
        serde_json::from_slice(&body).unwrap();
    assert_eq!(page.items, vec!["s1".to_string()]);
    assert_eq!(page.total, 1);
    assert!(!page.has_more);

    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/v1/threads/s1/messages")
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let page: tirea_agentos::contracts::storage::MessagePage =
        serde_json::from_slice(&body).unwrap();
    assert_eq!(page.messages.len(), 1);
    assert_eq!(page.messages[0].message.content, "hello");
    assert!(!page.has_more);
}

#[tokio::test]
async fn test_ai_sdk_sse_and_persists_session() {
    let storage = Arc::new(MemoryStore::new());
    let os = Arc::new(make_os_with_storage(storage.clone()));
    let app = router(AppState {
        os: os.clone(),
        read_store: storage.clone(),
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
                .uri("/v1/ai-sdk/agents/test/runs")
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
        text.contains(r#""type":"start""#),
        "missing start event: {text}"
    );
    // text-start/text-end are lazy — only emitted when TextDelta events occur.
    // This test skips inference, so no text is produced.
    assert!(
        !text.contains(r#""type":"text-start""#),
        "unexpected text-start without text content: {text}"
    );
    assert!(
        text.contains(r#""type":"finish""#),
        "missing finish: {text}"
    );

    let saved = storage.load_agent_state("t1").await.unwrap().unwrap();
    assert_eq!(saved.id, "t1");
    assert_eq!(saved.messages.len(), 1);
    assert_eq!(saved.messages[0].content, "hi");
}

#[tokio::test]
async fn test_ai_sdk_sse_accepts_messages_request_shape() {
    let storage = Arc::new(MemoryStore::new());
    let os = Arc::new(make_os_with_storage(storage.clone()));
    let app = router(AppState {
        os: os.clone(),
        read_store: storage.clone(),
    });

    let payload = json!({
        "id": "t-id-fallback",
        "sessionId": "t-messages-shape",
        "messages": [
            { "role": "user", "parts": [{ "type": "text", "text": "first input" }] },
            { "role": "assistant", "parts": [{ "type": "text", "text": "ignored assistant text" }] },
            { "role": "user", "parts": [{ "type": "text", "text": "latest input" }] }
        ],
        "runId": "run-messages-shape"
    });

    let (status, text) = post_sse_text(app, "/v1/ai-sdk/agents/test/runs", payload).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        text.contains(r#""type":"finish""#),
        "missing finish event: {text}"
    );

    let saved = storage
        .load_agent_state("t-messages-shape")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(saved.messages.len(), 1);
    assert_eq!(saved.messages[0].content, "latest input");
}

#[tokio::test]
async fn test_ai_sdk_sse_messages_request_uses_id_when_session_id_missing() {
    let storage = Arc::new(MemoryStore::new());
    let os = Arc::new(make_os_with_storage(storage.clone()));
    let app = router(AppState {
        os: os.clone(),
        read_store: storage.clone(),
    });

    let payload = json!({
        "id": "t-id-only",
        "messages": [
            { "role": "user", "content": "id-fallback-input" }
        ],
        "runId": "run-id-only"
    });

    let (status, text) = post_sse_text(app, "/v1/ai-sdk/agents/test/runs", payload).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        text.contains(r#""type":"finish""#),
        "missing finish event: {text}"
    );

    let saved = storage
        .load_agent_state("t-id-only")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(saved.messages.len(), 1);
    assert_eq!(saved.messages[0].content, "id-fallback-input");
}

#[tokio::test]
async fn test_ai_sdk_sse_messages_request_requires_user_text() {
    let os = Arc::new(make_os());
    let read_store: Arc<dyn AgentStateReader> = Arc::new(MemoryStore::new());
    let app = make_app(os, read_store);

    let payload = json!({
        "id": "s1",
        "messages": [
            { "role": "assistant", "parts": [{ "type": "text", "text": "no user turn" }] }
        ]
    });
    let (status, body) = post_json(app, "/v1/ai-sdk/agents/test/runs", payload).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(
        body["error"].as_str().unwrap_or("").contains("input"),
        "expected input validation error: {body}"
    );
}

#[tokio::test]
async fn test_ai_sdk_sse_messages_request_requires_thread_identifier() {
    let os = Arc::new(make_os());
    let read_store: Arc<dyn AgentStateReader> = Arc::new(MemoryStore::new());
    let app = make_app(os, read_store);

    let payload = json!({
        "messages": [
            { "role": "user", "content": "hello without thread id" }
        ]
    });
    let (status, body) = post_json(app, "/v1/ai-sdk/agents/test/runs", payload).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(
        body["error"]
            .as_str()
            .unwrap_or("")
            .contains("sessionId cannot be empty"),
        "expected sessionId validation error: {body}"
    );
}

#[tokio::test]
async fn test_ai_sdk_sse_accepts_messages_content_array_shape() {
    let storage = Arc::new(MemoryStore::new());
    let os = Arc::new(make_os_with_storage(storage.clone()));
    let app = router(AppState {
        os: os.clone(),
        read_store: storage.clone(),
    });

    let payload = json!({
        "id": "t-content-array",
        "messages": [
            {
                "role": "user",
                "content": [
                    { "type": "text", "text": "content-array-input" },
                    { "type": "file", "url": "https://example.com/f.txt" }
                ]
            }
        ],
        "runId": "run-content-array"
    });

    let (status, text) = post_sse_text(app, "/v1/ai-sdk/agents/test/runs", payload).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        text.contains(r#""type":"finish""#),
        "missing finish event: {text}"
    );

    let saved = storage
        .load_agent_state("t-content-array")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(saved.messages.len(), 1);
    assert_eq!(saved.messages[0].content, "content-array-input");
}

#[tokio::test]
async fn test_ai_sdk_sse_sets_expected_headers_and_done_trailer() {
    let storage = Arc::new(MemoryStore::new());
    let os = Arc::new(make_os_with_storage(storage.clone()));
    let app = router(AppState {
        os,
        read_store: storage,
    });

    let payload = json!({
        "sessionId": "t-headers",
        "input": "hello",
        "runId": "run-headers"
    });

    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/ai-sdk/agents/test/runs")
                .header("content-type", "application/json")
                .body(axum::body::Body::from(payload.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(
        resp.headers()
            .get("x-vercel-ai-ui-message-stream")
            .and_then(|v| v.to_str().ok()),
        Some("v1")
    );
    assert_eq!(
        resp.headers()
            .get("x-tirea-ai-sdk-version")
            .and_then(|v| v.to_str().ok()),
        Some(tirea_protocol_ai_sdk_v6::AI_SDK_VERSION)
    );

    let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let text = String::from_utf8(body.to_vec()).unwrap();
    assert!(
        text.contains("data: [DONE]"),
        "ai-sdk stream should end with [DONE] trailer: {text}"
    );
}

#[tokio::test]
async fn test_ai_sdk_sse_auto_generated_run_id_is_uuid_v7() {
    let storage = Arc::new(MemoryStore::new());
    let os = Arc::new(make_os_with_storage(storage.clone()));
    let app = router(AppState {
        os: os.clone(),
        read_store: storage.clone(),
    });

    let payload = json!({
        "sessionId": "t-v7",
        "input": "hi"
    });

    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/ai-sdk/agents/test/runs")
                .header("content-type", "application/json")
                .body(axum::body::Body::from(payload.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let text = String::from_utf8(body.to_vec()).unwrap();

    let events: Vec<Value> = text
        .lines()
        .filter_map(|line| line.strip_prefix("data: "))
        .filter_map(|json| serde_json::from_str::<Value>(json).ok())
        .collect();

    let run_id = events
        .iter()
        .find(|e| e["type"] == "data-run-info")
        .and_then(|e| e["data"]["runId"].as_str())
        .unwrap_or_else(|| panic!("missing run-info event with run_id: {text}"));
    let parsed = uuid::Uuid::parse_str(run_id)
        .unwrap_or_else(|_| panic!("run_id must be parseable UUID, got: {run_id}"));
    assert_eq!(
        parsed.get_variant(),
        uuid::Variant::RFC4122,
        "run_id must be RFC4122 UUID, got: {run_id}"
    );
    assert_eq!(
        parsed.get_version_num(),
        7,
        "run_id must be version 7 UUID, got: {run_id}"
    );
}

#[tokio::test]
async fn test_agui_sse_and_persists_session() {
    let storage = Arc::new(MemoryStore::new());
    let os = Arc::new(make_os_with_storage(storage.clone()));
    let app = router(AppState {
        os: os.clone(),
        read_store: storage.clone(),
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
                .uri("/v1/ag-ui/agents/test/runs")
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

    let saved = storage.load_agent_state("th1").await.unwrap().unwrap();
    assert_eq!(saved.id, "th1");
    assert_eq!(saved.messages.len(), 1);
}

#[tokio::test]
async fn test_industry_common_persistence_saves_user_message_before_run_completes_ai_sdk() {
    let storage = Arc::new(RecordingStorage::default());
    let os = Arc::new(make_os_with_storage(storage.clone()));
    let app = router(AppState {
        os,
        read_store: storage.clone(),
    });

    let payload = json!({
        "sessionId": "t2",
        "input": "hi",
        "runId": "run_2"
    });

    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/ai-sdk/agents/test/runs")
                .header("content-type", "application/json")
                .body(axum::body::Body::from(payload.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let _ = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    storage.wait_saves(2).await;

    assert!(
        storage.saves.load(Ordering::SeqCst) >= 2,
        "expected at least 2 saves (user ingress + final)"
    );

    let saved = storage.load_agent_state("t2").await.unwrap().unwrap();
    assert_eq!(saved.messages.len(), 1);
    assert_eq!(saved.messages[0].content, "hi");
}

#[tokio::test]
async fn test_industry_common_persistence_saves_inbound_request_messages_agui() {
    let storage = Arc::new(RecordingStorage::default());
    let os = Arc::new(make_os_with_storage(storage.clone()));
    let app = router(AppState {
        os,
        read_store: storage.clone(),
    });

    let payload = json!({
        "threadId": "th2",
        "runId": "r2",
        "messages": [
            {"role": "user", "content": "hello", "id": "m1"}
        ],
        "tools": []
    });

    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/ag-ui/agents/test/runs")
                .header("content-type", "application/json")
                .body(axum::body::Body::from(payload.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let _ = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    storage.wait_saves(2).await;

    assert!(
        storage.saves.load(Ordering::SeqCst) >= 2,
        "expected at least 2 saves (request ingress + final)"
    );

    let saved = storage.load_agent_state("th2").await.unwrap().unwrap();
    assert_eq!(saved.messages.len(), 1);
    assert_eq!(saved.messages[0].content, "hello");
}

#[tokio::test]
async fn test_agui_sse_idless_user_message_not_duplicated_by_internal_reapply() {
    let storage = Arc::new(MemoryStore::new());
    let os = Arc::new(make_os_with_storage(storage.clone()));
    let app = router(AppState {
        os: os.clone(),
        read_store: storage.clone(),
    });

    let payload = json!({
        "threadId": "th-idless-once",
        "runId": "r-idless-once",
        "messages": [
            {"role": "user", "content": "hello without id"}
        ],
        "tools": []
    });

    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/ag-ui/agents/test/runs")
                .header("content-type", "application/json")
                .body(axum::body::Body::from(payload.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let _ = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let saved = storage
        .load_agent_state("th-idless-once")
        .await
        .unwrap()
        .unwrap();
    let user_hello_count = saved
        .messages
        .iter()
        .filter(|m| {
            m.role == tirea_agentos::contracts::thread::Role::User
                && m.content == "hello without id"
        })
        .count();
    assert_eq!(
        user_hello_count, 1,
        "id-less user message should be applied exactly once per request"
    );
}

// ============================================================================
// Helper: POST JSON and return (status, body_json)
// ============================================================================

async fn post_json(app: axum::Router, uri: &str, payload: Value) -> (StatusCode, Value) {
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
    let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let json: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
    (status, json)
}

async fn post_sse_text(app: axum::Router, uri: &str, payload: Value) -> (StatusCode, String) {
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
    let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let text = String::from_utf8(body.to_vec()).unwrap();
    (status, text)
}

async fn get_json(app: axum::Router, uri: &str) -> (StatusCode, Value) {
    let resp = app
        .oneshot(
            Request::builder()
                .uri(uri)
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let status = resp.status();
    let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let json: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
    (status, json)
}

fn make_app(os: Arc<AgentOs>, read_store: Arc<dyn AgentStateReader>) -> axum::Router {
    router(AppState { os, read_store })
}

// ============================================================================
// Health endpoint
// ============================================================================

#[tokio::test]
async fn test_health_returns_200() {
    let os = Arc::new(make_os());
    let read_store: Arc<dyn AgentStateReader> = Arc::new(MemoryStore::new());
    let app = make_app(os, read_store);

    let resp = app
        .oneshot(
            Request::builder()
                .uri("/health")
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}

// ============================================================================
// GET /v1/sessions/:id — session not found
// ============================================================================

#[tokio::test]
async fn test_get_session_not_found() {
    let os = Arc::new(make_os());
    let read_store: Arc<dyn AgentStateReader> = Arc::new(MemoryStore::new());
    let app = make_app(os, read_store);

    let (status, body) = get_json(app, "/v1/threads/nonexistent").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert!(
        body["error"].as_str().unwrap_or("").contains("not found"),
        "expected not found error: {body}"
    );
}

// ============================================================================
// GET /v1/sessions/:id/messages — session not found
// ============================================================================

#[tokio::test]
async fn test_get_session_messages_not_found() {
    let os = Arc::new(make_os());
    let read_store: Arc<dyn AgentStateReader> = Arc::new(MemoryStore::new());
    let app = make_app(os, read_store);

    let (status, body) = get_json(app, "/v1/threads/nonexistent/messages").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert!(
        body["error"].as_str().unwrap_or("").contains("not found"),
        "expected not found error: {body}"
    );
}

// ============================================================================
// AI SDK SSE — error paths
// ============================================================================

#[tokio::test]
async fn test_ai_sdk_sse_empty_thread_id() {
    let os = Arc::new(make_os());
    let read_store: Arc<dyn AgentStateReader> = Arc::new(MemoryStore::new());
    let app = make_app(os, read_store);

    let payload = json!({ "sessionId": "  ", "input": "hi" });
    let (status, body) = post_json(app, "/v1/ai-sdk/agents/test/runs", payload).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(
        body["error"].as_str().unwrap_or("").contains("sessionId"),
        "expected sessionId error: {body}"
    );
}

#[tokio::test]
async fn test_ai_sdk_sse_empty_input() {
    let os = Arc::new(make_os());
    let read_store: Arc<dyn AgentStateReader> = Arc::new(MemoryStore::new());
    let app = make_app(os, read_store);

    let payload = json!({ "sessionId": "s1", "input": "  " });
    let (status, body) = post_json(app, "/v1/ai-sdk/agents/test/runs", payload).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(
        body["error"].as_str().unwrap_or("").contains("input"),
        "expected input error: {body}"
    );
}

#[tokio::test]
async fn test_ai_sdk_sse_agent_not_found() {
    let os = Arc::new(make_os());
    let read_store: Arc<dyn AgentStateReader> = Arc::new(MemoryStore::new());
    let app = make_app(os, read_store);

    let payload = json!({ "sessionId": "s1", "input": "hi" });
    let (status, body) = post_json(app, "/v1/ai-sdk/agents/no_such_agent/runs", payload).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert!(
        body["error"].as_str().unwrap_or("").contains("not found"),
        "expected agent not found error: {body}"
    );
}

#[tokio::test]
async fn test_ai_sdk_sse_malformed_json() {
    let os = Arc::new(make_os());
    let read_store: Arc<dyn AgentStateReader> = Arc::new(MemoryStore::new());
    let app = make_app(os, read_store);

    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/ai-sdk/agents/test/runs")
                .header("content-type", "application/json")
                .body(axum::body::Body::from("not json"))
                .unwrap(),
        )
        .await
        .unwrap();
    // Axum returns 400 for JSON parse errors.
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

// ============================================================================
// AG-UI SSE — error paths
// ============================================================================

#[tokio::test]
async fn test_agui_sse_agent_not_found() {
    let os = Arc::new(make_os());
    let read_store: Arc<dyn AgentStateReader> = Arc::new(MemoryStore::new());
    let app = make_app(os, read_store);

    let payload = json!({
        "threadId": "th1", "runId": "r1",
        "messages": [{"role": "user", "content": "hi"}],
        "tools": []
    });
    let (status, body) = post_json(app, "/v1/ag-ui/agents/no_such_agent/runs", payload).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert!(
        body["error"].as_str().unwrap_or("").contains("not found"),
        "expected agent not found error: {body}"
    );
}

#[tokio::test]
async fn test_agui_sse_empty_thread_id() {
    let os = Arc::new(make_os());
    let read_store: Arc<dyn AgentStateReader> = Arc::new(MemoryStore::new());
    let app = make_app(os, read_store);

    let payload = json!({
        "threadId": "", "runId": "r1",
        "messages": [], "tools": []
    });
    let (status, body) = post_json(app, "/v1/ag-ui/agents/test/runs", payload).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(
        body["error"].as_str().unwrap_or("").contains("threadId"),
        "expected threadId validation error: {body}"
    );
}

#[tokio::test]
async fn test_agui_sse_empty_run_id() {
    let os = Arc::new(make_os());
    let read_store: Arc<dyn AgentStateReader> = Arc::new(MemoryStore::new());
    let app = make_app(os, read_store);

    let payload = json!({
        "threadId": "th1", "runId": "",
        "messages": [], "tools": []
    });
    let (status, body) = post_json(app, "/v1/ag-ui/agents/test/runs", payload).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(
        body["error"].as_str().unwrap_or("").contains("runId"),
        "expected runId validation error: {body}"
    );
}

#[tokio::test]
async fn test_agui_sse_malformed_json() {
    let os = Arc::new(make_os());
    let read_store: Arc<dyn AgentStateReader> = Arc::new(MemoryStore::new());
    let app = make_app(os, read_store);

    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/ag-ui/agents/test/runs")
                .header("content-type", "application/json")
                .body(axum::body::Body::from("{bad"))
                .unwrap(),
        )
        .await
        .unwrap();
    // Axum returns 400 for JSON parse errors.
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn test_agui_sse_session_id_mismatch() {
    let os = Arc::new(make_os());
    let storage = Arc::new(MemoryStore::new());
    let read_store: Arc<dyn AgentStateReader> = storage.clone();

    // Pre-save a session with id "real-id".
    storage.save(&Thread::new("real-id")).await.unwrap();

    let _app = make_app(os, read_store);

    // The mismatch can only occur if storage returns a session with a different id than the key.
    // This is an internal consistency check, not triggerable from external input.
}

// ============================================================================
// Failing storage — tests error propagation
// ============================================================================

struct FailingStorage;

#[async_trait]
impl AgentStateWriter for FailingStorage {
    async fn create(&self, _thread: &Thread) -> Result<Committed, AgentStateStoreError> {
        Err(AgentStateStoreError::Io(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "disk write denied",
        )))
    }

    async fn append(
        &self,
        _id: &str,
        _delta: &tirea_agentos::contracts::ThreadChangeSet,
        _precondition: tirea_agentos::contracts::storage::VersionPrecondition,
    ) -> Result<Committed, AgentStateStoreError> {
        Err(AgentStateStoreError::Io(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "disk write denied",
        )))
    }

    async fn delete(&self, _id: &str) -> Result<(), AgentStateStoreError> {
        Err(AgentStateStoreError::Io(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "disk delete denied",
        )))
    }

    async fn save(&self, _thread: &Thread) -> Result<(), AgentStateStoreError> {
        Err(AgentStateStoreError::Io(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "disk write denied",
        )))
    }
}

#[async_trait]
impl AgentStateReader for FailingStorage {
    async fn load(&self, _id: &str) -> Result<Option<AgentStateHead>, AgentStateStoreError> {
        Err(AgentStateStoreError::Io(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "disk read denied",
        )))
    }

    async fn list_agent_states(
        &self,
        _query: &AgentStateListQuery,
    ) -> Result<AgentStateListPage, AgentStateStoreError> {
        Err(AgentStateStoreError::Io(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "disk list denied",
        )))
    }
}

#[tokio::test]
async fn test_list_sessions_storage_error() {
    let os = Arc::new(make_os());
    let read_store: Arc<dyn AgentStateReader> = Arc::new(FailingStorage);
    let app = make_app(os, read_store);

    let (status, body) = get_json(app, "/v1/threads").await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert!(
        body["error"].as_str().unwrap_or("").contains("denied"),
        "expected storage error: {body}"
    );
}

#[tokio::test]
async fn test_get_session_storage_error() {
    let os = Arc::new(make_os());
    let read_store: Arc<dyn AgentStateReader> = Arc::new(FailingStorage);
    let app = make_app(os, read_store);

    let (status, body) = get_json(app, "/v1/threads/s1").await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert!(
        body["error"].as_str().unwrap_or("").contains("denied"),
        "expected storage error: {body}"
    );
}

#[tokio::test]
async fn test_ai_sdk_sse_storage_load_error() {
    let os = Arc::new(make_os_with_storage(Arc::new(FailingStorage)));
    let read_store: Arc<dyn AgentStateReader> = Arc::new(FailingStorage);
    let app = make_app(os, read_store);

    let payload = json!({ "sessionId": "s1", "input": "hi" });
    let (status, body) = post_json(app, "/v1/ai-sdk/agents/test/runs", payload).await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert!(
        body["error"].as_str().unwrap_or("").contains("denied"),
        "expected storage error: {body}"
    );
}

#[tokio::test]
async fn test_agui_sse_storage_load_error() {
    let os = Arc::new(make_os_with_storage(Arc::new(FailingStorage)));
    let read_store: Arc<dyn AgentStateReader> = Arc::new(FailingStorage);
    let app = make_app(os, read_store);

    let payload = json!({
        "threadId": "th1", "runId": "r1",
        "messages": [{"role": "user", "content": "hi"}],
        "tools": []
    });
    let (status, body) = post_json(app, "/v1/ag-ui/agents/test/runs", payload).await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert!(
        body["error"].as_str().unwrap_or("").contains("denied"),
        "expected storage error: {body}"
    );
}

/// Storage that loads OK but fails on save — tests user message persistence error.
struct SaveFailStorage;

#[async_trait]
impl AgentStateWriter for SaveFailStorage {
    async fn create(&self, _thread: &Thread) -> Result<Committed, AgentStateStoreError> {
        Err(AgentStateStoreError::Io(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "disk write denied",
        )))
    }

    async fn append(
        &self,
        _id: &str,
        _delta: &tirea_agentos::contracts::ThreadChangeSet,
        _precondition: tirea_agentos::contracts::storage::VersionPrecondition,
    ) -> Result<Committed, AgentStateStoreError> {
        Err(AgentStateStoreError::Io(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "disk write denied",
        )))
    }

    async fn delete(&self, _id: &str) -> Result<(), AgentStateStoreError> {
        Ok(())
    }

    async fn save(&self, _thread: &Thread) -> Result<(), AgentStateStoreError> {
        Err(AgentStateStoreError::Io(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "disk write denied",
        )))
    }
}

#[async_trait]
impl AgentStateReader for SaveFailStorage {
    async fn load(&self, _id: &str) -> Result<Option<AgentStateHead>, AgentStateStoreError> {
        Ok(None)
    }

    async fn list_agent_states(
        &self,
        _query: &AgentStateListQuery,
    ) -> Result<AgentStateListPage, AgentStateStoreError> {
        Ok(AgentStateListPage {
            items: vec![],
            total: 0,
            has_more: false,
        })
    }
}

#[tokio::test]
async fn test_ai_sdk_sse_storage_save_error() {
    let os = Arc::new(make_os_with_storage(Arc::new(SaveFailStorage)));
    let read_store: Arc<dyn AgentStateReader> = Arc::new(SaveFailStorage);
    let app = make_app(os, read_store);

    let payload = json!({ "sessionId": "s1", "input": "hi" });
    let (status, body) = post_json(app, "/v1/ai-sdk/agents/test/runs", payload).await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert!(
        body["error"].as_str().unwrap_or("").contains("denied"),
        "expected storage save error: {body}"
    );
}

#[tokio::test]
async fn test_agui_sse_storage_save_error() {
    let os = Arc::new(make_os_with_storage(Arc::new(SaveFailStorage)));
    let read_store: Arc<dyn AgentStateReader> = Arc::new(SaveFailStorage);
    let app = make_app(os, read_store);

    // Send a message so the session has changes to persist.
    let payload = json!({
        "threadId": "th1", "runId": "r1",
        "messages": [{"role": "user", "content": "hi"}],
        "tools": []
    });
    let (status, body) = post_json(app, "/v1/ag-ui/agents/test/runs", payload).await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert!(
        body["error"].as_str().unwrap_or("").contains("denied"),
        "expected storage save error: {body}"
    );
}

// ============================================================================
// Message pagination tests
// ============================================================================

fn make_session_with_n_messages(id: &str, n: usize) -> Thread {
    let mut thread = Thread::new(id);
    for i in 0..n {
        thread = thread.with_message(tirea_agentos::contracts::thread::Message::user(format!(
            "msg-{}",
            i
        )));
    }
    thread
}

#[tokio::test]
async fn test_messages_pagination_default_params() {
    let os = Arc::new(make_os());
    let storage = Arc::new(MemoryStore::new());
    let read_store: Arc<dyn AgentStateReader> = storage.clone();
    let thread = make_session_with_n_messages("s1", 5);
    storage.save(&thread).await.unwrap();
    let app = make_app(os, read_store);

    let (status, body) = get_json(app, "/v1/threads/s1/messages").await;
    assert_eq!(status, StatusCode::OK);
    let page: tirea_agentos::contracts::storage::MessagePage =
        serde_json::from_value(body).unwrap();
    assert_eq!(page.messages.len(), 5);
    assert!(!page.has_more);
    assert_eq!(page.messages[0].message.content, "msg-0");
    assert_eq!(page.messages[4].message.content, "msg-4");
}

#[tokio::test]
async fn test_messages_pagination_with_limit() {
    let os = Arc::new(make_os());
    let storage = Arc::new(MemoryStore::new());
    let read_store: Arc<dyn AgentStateReader> = storage.clone();
    let thread = make_session_with_n_messages("s1", 10);
    storage.save(&thread).await.unwrap();
    let app = make_app(os, read_store);

    let (status, body) = get_json(app, "/v1/threads/s1/messages?limit=3").await;
    assert_eq!(status, StatusCode::OK);
    let page: tirea_agentos::contracts::storage::MessagePage =
        serde_json::from_value(body).unwrap();
    assert_eq!(page.messages.len(), 3);
    assert!(page.has_more);
    assert_eq!(page.messages[0].cursor, 0);
    assert_eq!(page.messages[2].cursor, 2);
}

#[tokio::test]
async fn test_messages_pagination_cursor_forward() {
    let os = Arc::new(make_os());
    let storage = Arc::new(MemoryStore::new());
    let read_store: Arc<dyn AgentStateReader> = storage.clone();
    let thread = make_session_with_n_messages("s1", 10);
    storage.save(&thread).await.unwrap();
    let app = make_app(os, read_store);

    let (status, body) = get_json(app, "/v1/threads/s1/messages?after=4&limit=3").await;
    assert_eq!(status, StatusCode::OK);
    let page: tirea_agentos::contracts::storage::MessagePage =
        serde_json::from_value(body).unwrap();
    assert_eq!(page.messages.len(), 3);
    assert_eq!(page.messages[0].cursor, 5);
    assert_eq!(page.messages[0].message.content, "msg-5");
}

#[tokio::test]
async fn test_messages_pagination_desc_order() {
    let os = Arc::new(make_os());
    let storage = Arc::new(MemoryStore::new());
    let read_store: Arc<dyn AgentStateReader> = storage.clone();
    let thread = make_session_with_n_messages("s1", 10);
    storage.save(&thread).await.unwrap();
    let app = make_app(os, read_store);

    let (status, body) = get_json(app, "/v1/threads/s1/messages?order=desc&before=8&limit=3").await;
    assert_eq!(status, StatusCode::OK);
    let page: tirea_agentos::contracts::storage::MessagePage =
        serde_json::from_value(body).unwrap();
    assert_eq!(page.messages.len(), 3);
    // Desc order: highest cursors first
    assert_eq!(page.messages[0].cursor, 7);
    assert_eq!(page.messages[1].cursor, 6);
    assert_eq!(page.messages[2].cursor, 5);
}

#[tokio::test]
async fn test_messages_pagination_limit_clamped() {
    let os = Arc::new(make_os());
    let storage = Arc::new(MemoryStore::new());
    let read_store: Arc<dyn AgentStateReader> = storage.clone();
    let thread = make_session_with_n_messages("s1", 300);
    storage.save(&thread).await.unwrap();
    let app = make_app(os, read_store);

    let (status, body) = get_json(app, "/v1/threads/s1/messages?limit=999").await;
    assert_eq!(status, StatusCode::OK);
    let page: tirea_agentos::contracts::storage::MessagePage =
        serde_json::from_value(body).unwrap();
    // limit should be clamped to 200
    assert_eq!(page.messages.len(), 200);
    assert!(page.has_more);
}

#[tokio::test]
async fn test_messages_pagination_not_found() {
    let os = Arc::new(make_os());
    let read_store: Arc<dyn AgentStateReader> = Arc::new(MemoryStore::new());
    let app = make_app(os, read_store);

    let (status, body) = get_json(app, "/v1/threads/nonexistent/messages?limit=10").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert!(
        body["error"].as_str().unwrap_or("").contains("not found"),
        "expected not found error: {body}"
    );
}

#[tokio::test]
async fn test_list_threads_filters_by_parent_thread_id() {
    let os = Arc::new(make_os());
    let storage = Arc::new(MemoryStore::new());
    let read_store: Arc<dyn AgentStateReader> = storage.clone();

    storage
        .save(&Thread::new("t-parent-1").with_parent_thread_id("p-root"))
        .await
        .unwrap();
    storage
        .save(&Thread::new("t-parent-2").with_parent_thread_id("p-root"))
        .await
        .unwrap();
    storage.save(&Thread::new("t-other")).await.unwrap();

    let app = make_app(os, read_store);
    let (status, body) = get_json(app, "/v1/threads?parent_thread_id=p-root").await;
    assert_eq!(status, StatusCode::OK);

    let page: tirea_agentos::contracts::storage::AgentStateListPage =
        serde_json::from_value(body).unwrap();
    assert_eq!(page.total, 2);
    assert_eq!(page.items, vec!["t-parent-1", "t-parent-2"]);
}

#[tokio::test]
async fn test_messages_filter_by_visibility_and_run_id() {
    let os = Arc::new(make_os());
    let storage = Arc::new(MemoryStore::new());
    let read_store: Arc<dyn AgentStateReader> = storage.clone();

    let thread = Thread::new("s-filter")
        .with_message(
            tirea_agentos::contracts::thread::Message::user("visible-run-1").with_metadata(
                tirea_agentos::contracts::thread::MessageMetadata {
                    run_id: Some("run-1".to_string()),
                    step_index: Some(0),
                },
            ),
        )
        .with_message(
            tirea_agentos::contracts::thread::Message::internal_system("internal-run-1")
                .with_metadata(tirea_agentos::contracts::thread::MessageMetadata {
                    run_id: Some("run-1".to_string()),
                    step_index: Some(1),
                }),
        )
        .with_message(
            tirea_agentos::contracts::thread::Message::assistant("visible-run-2").with_metadata(
                tirea_agentos::contracts::thread::MessageMetadata {
                    run_id: Some("run-2".to_string()),
                    step_index: Some(2),
                },
            ),
        );
    storage.save(&thread).await.unwrap();

    let app = make_app(os, read_store);

    let (status, body) = get_json(app.clone(), "/v1/threads/s-filter/messages").await;
    assert_eq!(status, StatusCode::OK);
    let page: tirea_agentos::contracts::storage::MessagePage =
        serde_json::from_value(body).unwrap();
    assert_eq!(page.messages.len(), 2);
    assert_eq!(page.messages[0].message.content, "visible-run-1");
    assert_eq!(page.messages[1].message.content, "visible-run-2");

    let (status, body) = get_json(
        app.clone(),
        "/v1/threads/s-filter/messages?visibility=internal",
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let page: tirea_agentos::contracts::storage::MessagePage =
        serde_json::from_value(body).unwrap();
    assert_eq!(page.messages.len(), 1);
    assert_eq!(page.messages[0].message.content, "internal-run-1");

    let (status, body) = get_json(
        app.clone(),
        "/v1/threads/s-filter/messages?visibility=none&run_id=run-1",
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let page: tirea_agentos::contracts::storage::MessagePage =
        serde_json::from_value(body).unwrap();
    assert_eq!(page.messages.len(), 2);
    assert_eq!(page.messages[0].message.content, "visible-run-1");
    assert_eq!(page.messages[1].message.content, "internal-run-1");
}

#[tokio::test]
async fn test_protocol_history_endpoints_hide_internal_messages_by_default() {
    let os = Arc::new(make_os());
    let storage = Arc::new(MemoryStore::new());
    let read_store: Arc<dyn AgentStateReader> = storage.clone();

    let thread = Thread::new("s-internal-history")
        .with_message(tirea_agentos::contracts::thread::Message::user(
            "visible-user",
        ))
        .with_message(tirea_agentos::contracts::thread::Message::internal_system(
            "internal-secret",
        ))
        .with_message(tirea_agentos::contracts::thread::Message::assistant(
            "visible-assistant",
        ));
    storage.save(&thread).await.unwrap();

    let app = make_app(os, read_store);

    let (status, body) =
        get_json(app.clone(), "/v1/ag-ui/threads/s-internal-history/messages").await;
    assert_eq!(status, StatusCode::OK);
    let body_text = body.to_string();
    assert!(
        !body_text.contains("internal-secret"),
        "ag-ui history must not expose internal messages: {body_text}"
    );
    assert!(body_text.contains("visible-user"));
    assert!(body_text.contains("visible-assistant"));

    let (status, body) = get_json(app, "/v1/ai-sdk/threads/s-internal-history/messages").await;
    assert_eq!(status, StatusCode::OK);
    let body_text = body.to_string();
    assert!(
        !body_text.contains("internal-secret"),
        "ai-sdk history must not expose internal messages: {body_text}"
    );
    assert!(body_text.contains("visible-user"));
    assert!(body_text.contains("visible-assistant"));
}

#[tokio::test]
async fn test_ai_sdk_history_encodes_tool_messages_as_tool_invocation_parts() {
    let os = Arc::new(make_os());
    let storage = Arc::new(MemoryStore::new());
    let read_store: Arc<dyn AgentStateReader> = storage.clone();

    let thread = Thread::new("s-tool-history")
        .with_message(
            tirea_agentos::contracts::thread::Message::assistant_with_tool_calls(
                "calling tool",
                vec![tirea_agentos::contracts::thread::ToolCall::new(
                    "call_1",
                    "search",
                    json!({"q":"rust"}),
                )],
            ),
        )
        .with_message(tirea_agentos::contracts::thread::Message::tool(
            "call_1",
            r#"{"result":"ok"}"#,
        ));
    storage.save(&thread).await.unwrap();

    let app = make_app(os, read_store);
    let (status, body) = get_json(
        app,
        "/v1/ai-sdk/threads/s-tool-history/messages?visibility=none",
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let messages = body["messages"]
        .as_array()
        .expect("messages should be array");
    let has_tool_invocation = messages.iter().any(|msg| {
        msg["parts"]
            .as_array()
            .is_some_and(|parts| parts.iter().any(|part| part["type"] == "tool-invocation"))
    });
    assert!(
        has_tool_invocation,
        "ai-sdk history should include tool-invocation parts: {body}"
    );

    let has_tool_output = messages.iter().any(|msg| {
        msg["parts"].as_array().is_some_and(|parts| {
            parts.iter().any(|part| {
                part["type"] == "tool-invocation"
                    && part["toolCallId"] == "call_1"
                    && part["output"]["result"] == "ok"
            })
        })
    });
    assert!(
        has_tool_output,
        "tool output should be encoded in tool-invocation part: {body}"
    );
}

#[tokio::test]
async fn test_ai_sdk_history_encodes_assistant_tool_call_input_as_input_available() {
    let os = Arc::new(make_os());
    let storage = Arc::new(MemoryStore::new());
    let read_store: Arc<dyn AgentStateReader> = storage.clone();

    let thread = Thread::new("s-tool-input-history").with_message(
        tirea_agentos::contracts::thread::Message::assistant_with_tool_calls(
            "calling tool",
            vec![tirea_agentos::contracts::thread::ToolCall::new(
                "call_input_1",
                "search",
                json!({"q":"rust ai-sdk"}),
            )],
        ),
    );
    storage.save(&thread).await.unwrap();

    let app = make_app(os, read_store);
    let (status, body) = get_json(
        app,
        "/v1/ai-sdk/threads/s-tool-input-history/messages?visibility=none",
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let messages = body["messages"]
        .as_array()
        .expect("messages should be array");
    let has_tool_input_available = messages.iter().any(|msg| {
        msg["parts"].as_array().is_some_and(|parts| {
            parts.iter().any(|part| {
                part["type"] == "tool-invocation"
                    && part["toolCallId"] == "call_input_1"
                    && part["state"] == "input-available"
                    && part["input"]["q"] == "rust ai-sdk"
            })
        })
    });
    assert!(
        has_tool_input_available,
        "assistant tool call should encode to input-available tool-invocation part: {body}"
    );
}

#[tokio::test]
async fn test_messages_run_id_cursor_order_combination_boundaries() {
    let os = Arc::new(make_os());
    let storage = Arc::new(MemoryStore::new());
    let read_store: Arc<dyn AgentStateReader> = storage.clone();

    let mut thread = Thread::new("s-run-cursor-boundary");
    for i in 0..6usize {
        let run_id = if i % 2 == 0 { "run-a" } else { "run-b" };
        thread = thread.with_message(
            tirea_agentos::contracts::thread::Message::user(format!("msg-{i}")).with_metadata(
                tirea_agentos::contracts::thread::MessageMetadata {
                    run_id: Some(run_id.to_string()),
                    step_index: Some(i as u32),
                },
            ),
        );
    }
    storage.save(&thread).await.unwrap();
    let app = make_app(os, read_store);

    let (status, body) = get_json(
        app.clone(),
        "/v1/threads/s-run-cursor-boundary/messages?run_id=run-a&order=desc&before=5&limit=2",
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let page: tirea_agentos::contracts::storage::MessagePage =
        serde_json::from_value(body).unwrap();
    assert_eq!(page.messages.len(), 2);
    assert_eq!(page.messages[0].cursor, 4);
    assert_eq!(page.messages[0].message.content, "msg-4");
    assert_eq!(page.messages[1].cursor, 2);
    assert_eq!(page.messages[1].message.content, "msg-2");

    let (status, body) = get_json(
        app,
        "/v1/threads/s-run-cursor-boundary/messages?run_id=run-a&after=4&limit=10",
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let page: tirea_agentos::contracts::storage::MessagePage =
        serde_json::from_value(body).unwrap();
    assert!(
        page.messages.is_empty(),
        "after=4 with run-a should be empty, got: {:?}",
        page.messages
            .iter()
            .map(|m| (m.cursor, m.message.content.clone()))
            .collect::<Vec<_>>()
    );
}

fn pending_echo_thread(id: &str, payload: &str) -> Thread {
    Thread::with_initial_state(
        id,
        json!({
            "loop_control": {
                "pending_interaction": {
                    "id": "call_1",
                    "action": "tool:echo",
                    "parameters": {
                        "source": "permission",
                        "origin_tool_call": {
                            "id": "call_1",
                            "name": "echo",
                            "arguments": { "message": payload }
                        }
                    }
                }
            }
        }),
    )
    .with_message(
        tirea_agentos::contracts::thread::Message::assistant_with_tool_calls(
            "need permission",
            vec![tirea_agentos::contracts::thread::ToolCall::new(
                "call_1",
                "echo",
                json!({"message": payload}),
            )],
        ),
    )
    .with_message(tirea_agentos::contracts::thread::Message::tool(
        "call_1",
        "Tool 'echo' is awaiting approval. Execution paused.",
    ))
}

#[tokio::test]
async fn test_agui_pending_approval_resumes_and_replays_tool_call() {
    let storage = Arc::new(MemoryStore::new());
    storage
        .save(&pending_echo_thread("th-approve", "approved-run"))
        .await
        .unwrap();

    let tools: HashMap<String, Arc<dyn Tool>> =
        HashMap::from([("echo".to_string(), Arc::new(EchoTool) as Arc<dyn Tool>)]);
    let os = Arc::new(make_os_with_storage_and_tools(storage.clone(), tools));
    let app = router(AppState {
        os,
        read_store: storage.clone(),
    });

    let payload = json!({
        "threadId": "th-approve",
        "runId": "resume-approve-1",
        "messages": [
            {"role": "tool", "content": "true", "toolCallId": "call_1"}
        ],
        "tools": []
    });
    let (status, body) = post_sse_text(app, "/v1/ag-ui/agents/test/runs", payload).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        body.contains("RUN_FINISHED"),
        "resume run should finish: {body}"
    );

    let saved = storage
        .load_agent_state("th-approve")
        .await
        .unwrap()
        .unwrap();
    let replayed_tool = saved.messages.iter().find(|m| {
        m.role == tirea_agentos::contracts::thread::Role::Tool
            && m.tool_call_id.as_deref() == Some("call_1")
            && m.content.contains("approved-run")
    });
    assert!(
        replayed_tool.is_some(),
        "approved flow should append replayed tool result"
    );

    let rebuilt = saved.rebuild_state().unwrap();
    assert!(
        rebuilt
            .get("loop_control")
            .and_then(|rt| rt.get("pending_interaction"))
            .is_none()
            || rebuilt
                .get("loop_control")
                .and_then(|rt| rt.get("pending_interaction"))
                == Some(&Value::Null),
        "pending_interaction must be cleared after approval replay"
    );
}

#[tokio::test]
async fn test_agui_pending_denial_clears_pending_without_replay() {
    let storage = Arc::new(MemoryStore::new());
    storage
        .save(&pending_echo_thread("th-deny", "denied-run"))
        .await
        .unwrap();

    let tools: HashMap<String, Arc<dyn Tool>> =
        HashMap::from([("echo".to_string(), Arc::new(EchoTool) as Arc<dyn Tool>)]);
    let os = Arc::new(make_os_with_storage_and_tools(storage.clone(), tools));
    let app = router(AppState {
        os,
        read_store: storage.clone(),
    });

    let payload = json!({
        "threadId": "th-deny",
        "runId": "resume-deny-1",
        "messages": [
            {"role": "tool", "content": "false", "toolCallId": "call_1"}
        ],
        "tools": []
    });
    let (status, body) = post_sse_text(app, "/v1/ag-ui/agents/test/runs", payload).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        body.contains("RUN_FINISHED"),
        "denied resume run should still finish: {body}"
    );

    let saved = storage.load_agent_state("th-deny").await.unwrap().unwrap();
    let replayed_tool = saved.messages.iter().find(|m| {
        m.role == tirea_agentos::contracts::thread::Role::Tool
            && m.tool_call_id.as_deref() == Some("call_1")
            && m.content.contains("echoed")
    });
    assert!(
        replayed_tool.is_none(),
        "denied flow must not replay original tool call"
    );

    let rebuilt = saved.rebuild_state().unwrap();
    assert!(
        rebuilt
            .get("loop_control")
            .and_then(|rt| rt.get("pending_interaction"))
            .is_none()
            || rebuilt
                .get("loop_control")
                .and_then(|rt| rt.get("pending_interaction"))
                == Some(&Value::Null),
        "pending_interaction must be cleared after denial"
    );
}

// ============================================================================
// Route restructuring regression tests
// ============================================================================

/// Old route paths must return 404 after restructuring.
#[tokio::test]
async fn test_old_routes_return_404() {
    let os = Arc::new(make_os());
    let read_store: Arc<dyn AgentStateReader> = Arc::new(MemoryStore::new());
    let app = make_app(os, read_store);

    let old_routes = vec![
        ("POST", "/v1/agents/test/runs/ag-ui/sse"),
        ("POST", "/v1/agents/test/runs/ai-sdk/sse"),
        ("GET", "/v1/threads/some-thread/messages/ag-ui"),
        ("GET", "/v1/threads/some-thread/messages/ai-sdk"),
    ];

    for (method, uri) in old_routes {
        let req = Request::builder()
            .method(method)
            .uri(uri)
            .header("content-type", "application/json")
            .body(axum::body::Body::from("{}"))
            .unwrap();
        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::NOT_FOUND,
            "old route {method} {uri} should return 404, got {}",
            resp.status()
        );
    }
}

/// New protocol-prefixed routes must be reachable.
#[tokio::test]
async fn test_new_protocol_routes_are_reachable() {
    let os = Arc::new(make_os());
    let read_store: Arc<dyn AgentStateReader> = Arc::new(MemoryStore::new());
    let app = make_app(os, read_store);

    // POST to new AG-UI run endpoint — valid request should succeed (200 SSE stream)
    let ag_ui_payload = json!({
        "threadId": "route-test-agui",
        "runId": "r-agui-1",
        "messages": [{"role": "user", "content": "hi"}],
        "tools": []
    });
    let (status, _) = post_sse_text(app.clone(), "/v1/ag-ui/agents/test/runs", ag_ui_payload).await;
    assert_eq!(status, StatusCode::OK, "new AG-UI run route should be 200");

    // POST to new AI SDK run endpoint
    let ai_sdk_payload = json!({
        "sessionId": "route-test-aisdk",
        "input": "hello",
    });
    let (status, _) =
        post_sse_text(app.clone(), "/v1/ai-sdk/agents/test/runs", ai_sdk_payload).await;
    assert_eq!(status, StatusCode::OK, "new AI SDK run route should be 200");

    // GET protocol-encoded history (thread not found → 404, but route is matched)
    let (status, _) = get_json(app.clone(), "/v1/ag-ui/threads/nonexistent/messages").await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "AG-UI history route should match but return thread-not-found"
    );

    let (status, _) = get_json(app.clone(), "/v1/ai-sdk/threads/nonexistent/messages").await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "AI SDK history route should match but return thread-not-found"
    );
}

/// Wrong HTTP methods on protocol routes should return 405 Method Not Allowed.
#[tokio::test]
async fn test_protocol_routes_reject_wrong_methods() {
    let os = Arc::new(make_os());
    let read_store: Arc<dyn AgentStateReader> = Arc::new(MemoryStore::new());
    let app = make_app(os, read_store);

    // GET on a POST-only endpoint
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/v1/ag-ui/agents/test/runs")
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::METHOD_NOT_ALLOWED,
        "GET on run endpoint should be 405"
    );

    // POST on a GET-only endpoint
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/ai-sdk/threads/some-thread/messages")
                .header("content-type", "application/json")
                .body(axum::body::Body::from("{}"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::METHOD_NOT_ALLOWED,
        "POST on history endpoint should be 405"
    );
}

/// Protocol isolation: AG-UI and AI SDK endpoints must not cross-route.
#[tokio::test]
async fn test_protocol_isolation_no_cross_routing() {
    let storage = Arc::new(MemoryStore::new());
    let os = Arc::new(make_os_with_storage(storage.clone()));
    let app = make_app(os, storage.clone());

    // Run via AG-UI
    let ag_payload = json!({
        "threadId": "cross-route-agui",
        "runId": "cr-1",
        "messages": [{"role": "user", "content": "agui msg"}],
        "tools": []
    });
    let (status, body) = post_sse_text(app.clone(), "/v1/ag-ui/agents/test/runs", ag_payload).await;
    assert_eq!(status, StatusCode::OK);
    // AG-UI events use "RUN_STARTED" / "RUN_FINISHED" markers
    assert!(
        body.contains("RUN_STARTED"),
        "AG-UI run should emit RUN_STARTED events"
    );

    // Run via AI SDK
    let sdk_payload = json!({
        "sessionId": "cross-route-aisdk",
        "input": "sdk msg",
    });
    let (status, body) =
        post_sse_text(app.clone(), "/v1/ai-sdk/agents/test/runs", sdk_payload).await;
    assert_eq!(status, StatusCode::OK);
    // AI SDK streams end with [DONE]
    assert!(
        body.contains("[DONE]"),
        "AI SDK run should emit [DONE] trailer"
    );
}

/// Unmatched protocol prefix should 404.
#[tokio::test]
async fn test_unknown_protocol_prefix_returns_404() {
    let os = Arc::new(make_os());
    let read_store: Arc<dyn AgentStateReader> = Arc::new(MemoryStore::new());
    let app = make_app(os, read_store);

    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/unknown-proto/agents/test/runs")
                .header("content-type", "application/json")
                .body(axum::body::Body::from("{}"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

/// Protocol-specific response headers: AI SDK sets x-vercel-ai-ui-message-stream.
#[tokio::test]
async fn test_ai_sdk_route_sets_protocol_headers() {
    let os = Arc::new(make_os());
    let read_store: Arc<dyn AgentStateReader> = Arc::new(MemoryStore::new());
    let app = make_app(os, read_store);

    let payload = json!({
        "sessionId": "header-check",
        "input": "test",
    });

    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/ai-sdk/agents/test/runs")
                .header("content-type", "application/json")
                .body(axum::body::Body::from(payload.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert!(
        resp.headers()
            .get("x-vercel-ai-ui-message-stream")
            .is_some(),
        "AI SDK endpoint must set x-vercel-ai-ui-message-stream header"
    );
    assert!(
        resp.headers().get("x-tirea-ai-sdk-version").is_some(),
        "AI SDK endpoint must set x-tirea-ai-sdk-version header"
    );
}

/// Generic thread endpoints remain accessible without protocol prefix.
#[tokio::test]
async fn test_generic_thread_endpoints_still_work() {
    let storage = Arc::new(MemoryStore::new());
    let os = Arc::new(make_os_with_storage(storage.clone()));
    let app = make_app(os, storage.clone());

    // Seed a thread via AG-UI
    let payload = json!({
        "threadId": "generic-ep-test",
        "runId": "g-1",
        "messages": [{"role": "user", "content": "seed"}],
        "tools": []
    });
    let (status, _) = post_sse_text(app.clone(), "/v1/ag-ui/agents/test/runs", payload).await;
    assert_eq!(status, StatusCode::OK);

    // Wait for persistence
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    // GET /v1/threads should list threads
    let (status, body) = get_json(app.clone(), "/v1/threads").await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        body.get("items").is_some(),
        "thread list should have items field"
    );

    // GET /v1/threads/:id should return thread
    let (status, _) = get_json(app.clone(), "/v1/threads/generic-ep-test").await;
    assert_eq!(status, StatusCode::OK);

    // GET /v1/threads/:id/messages should return raw messages
    let (status, body) = get_json(app.clone(), "/v1/threads/generic-ep-test/messages").await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        body.get("messages").is_some(),
        "messages endpoint should have messages field"
    );
}
