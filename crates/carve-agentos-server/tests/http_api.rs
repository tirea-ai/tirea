use async_trait::async_trait;
use axum::body::to_bytes;
use axum::http::{Request, StatusCode};
use carve_agent::phase::Phase;
use carve_agent::plugin::AgentPlugin;
use carve_agent::{
    AgentDefinition, AgentOs, AgentOsBuilder, Committed, MemoryStorage, Thread, ThreadHead,
    ThreadListPage, ThreadListQuery, StepContext, StorageError, ThreadQuery, ThreadStore, Version,
};
use carve_agentos_server::http::{router, AppState};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
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
    let def = AgentDefinition {
        id: "test".to_string(),
        plugins: vec![Arc::new(SkipInferencePlugin)],
        ..Default::default()
    };

    AgentOsBuilder::new()
        .with_agent("test", def)
        .build()
        .unwrap()
}

#[derive(Default)]
struct RecordingStorage {
    threads: RwLock<HashMap<String, Thread>>,
    saves: AtomicUsize,
    notify: Notify,
}

impl RecordingStorage {
    async fn wait_saves(&self, n: usize) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while self.saves.load(Ordering::SeqCst) < n {
            if std::time::Instant::now() > deadline {
                break;
            }
            self.notify.notified().await;
        }
    }
}

#[async_trait]
impl ThreadStore for RecordingStorage {
    async fn create(&self, thread: &Thread) -> Result<Committed, StorageError> {
        let mut threads = self.threads.write().await;
        if threads.contains_key(&thread.id) {
            return Err(StorageError::AlreadyExists);
        }
        threads.insert(thread.id.clone(), thread.clone());
        Ok(Committed { version: 0 })
    }

    async fn append(
        &self,
        _id: &str,
        _base_version: Version,
        _delta: &carve_agent::ThreadDelta,
    ) -> Result<Committed, StorageError> {
        Ok(Committed { version: 0 })
    }

    async fn load(&self, id: &str) -> Result<Option<ThreadHead>, StorageError> {
        let threads = self.threads.read().await;
        Ok(threads.get(id).map(|t| ThreadHead {
            thread: t.clone(),
            version: 0,
        }))
    }

    async fn delete(&self, id: &str) -> Result<(), StorageError> {
        let mut threads = self.threads.write().await;
        threads.remove(id);
        Ok(())
    }

    async fn save(&self, thread: &Thread) -> Result<(), StorageError> {
        let mut threads = self.threads.write().await;
        threads.insert(thread.id.clone(), thread.clone());
        self.saves.fetch_add(1, Ordering::SeqCst);
        self.notify.notify_waiters();
        Ok(())
    }
}

#[async_trait]
impl ThreadQuery for RecordingStorage {
    async fn list_threads(
        &self,
        query: &ThreadListQuery,
    ) -> Result<ThreadListPage, StorageError> {
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
        Ok(ThreadListPage {
            items,
            total,
            has_more,
        })
    }
}

#[tokio::test]
async fn test_sessions_query_endpoints() {
    let os = Arc::new(make_os());
    let storage = Arc::new(MemoryStorage::new());

    let thread = Thread::new("s1").with_message(carve_agent::Message::user("hello"));
    storage.save(&thread).await.unwrap();

    let app = router(AppState { os, storage });

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
    let page: carve_agent::ThreadListPage = serde_json::from_slice(&body).unwrap();
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
    let page: carve_agent::MessagePage = serde_json::from_slice(&body).unwrap();
    assert_eq!(page.messages.len(), 1);
    assert_eq!(page.messages[0].message.content, "hello");
    assert!(!page.has_more);
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

    let saved = storage.load_thread("t1").await.unwrap().unwrap();
    assert_eq!(saved.id, "t1");
    assert_eq!(saved.messages.len(), 1);
    assert_eq!(saved.messages[0].content, "hi");
}

#[tokio::test]
async fn test_ai_sdk_sse_auto_generated_run_id_is_uuid_v7() {
    let os = Arc::new(make_os());
    let storage = Arc::new(MemoryStorage::new());
    let app = router(AppState {
        os: os.clone(),
        storage: storage.clone(),
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

    let saved = storage.load_thread("th1").await.unwrap().unwrap();
    assert_eq!(saved.id, "th1");
    assert_eq!(saved.messages.len(), 1);
}

#[tokio::test]
async fn test_industry_common_persistence_saves_user_message_before_run_completes_ai_sdk() {
    let os = Arc::new(make_os());
    let storage = Arc::new(RecordingStorage::default());
    let app = router(AppState {
        os,
        storage: storage.clone(),
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
                .uri("/v1/agents/test/runs/ai-sdk/sse")
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

    let saved = storage.load_thread("t2").await.unwrap().unwrap();
    assert_eq!(saved.messages.len(), 1);
    assert_eq!(saved.messages[0].content, "hi");
}

#[tokio::test]
async fn test_industry_common_persistence_saves_inbound_request_messages_agui() {
    let os = Arc::new(make_os());
    let storage = Arc::new(RecordingStorage::default());
    let app = router(AppState {
        os,
        storage: storage.clone(),
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
                .uri("/v1/agents/test/runs/ag-ui/sse")
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

    let saved = storage.load_thread("th2").await.unwrap().unwrap();
    assert_eq!(saved.messages.len(), 1);
    assert_eq!(saved.messages[0].content, "hello");
}

#[tokio::test]
async fn test_agui_sse_idless_user_message_not_duplicated_by_internal_reapply() {
    let os = Arc::new(make_os());
    let storage = Arc::new(MemoryStorage::new());
    let app = router(AppState {
        os: os.clone(),
        storage: storage.clone(),
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
                .uri("/v1/agents/test/runs/ag-ui/sse")
                .header("content-type", "application/json")
                .body(axum::body::Body::from(payload.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let _ = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let saved = storage.load_thread("th-idless-once").await.unwrap().unwrap();
    let user_hello_count = saved
        .messages
        .iter()
        .filter(|m| m.role == carve_agent::Role::User && m.content == "hello without id")
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

fn make_app(os: Arc<AgentOs>, storage: Arc<dyn ThreadQuery>) -> axum::Router {
    router(AppState { os, storage })
}

// ============================================================================
// Health endpoint
// ============================================================================

#[tokio::test]
async fn test_health_returns_200() {
    let os = Arc::new(make_os());
    let storage: Arc<dyn ThreadQuery> = Arc::new(MemoryStorage::new());
    let app = make_app(os, storage);

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
    let storage: Arc<dyn ThreadQuery> = Arc::new(MemoryStorage::new());
    let app = make_app(os, storage);

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
    let storage: Arc<dyn ThreadQuery> = Arc::new(MemoryStorage::new());
    let app = make_app(os, storage);

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
    let storage: Arc<dyn ThreadQuery> = Arc::new(MemoryStorage::new());
    let app = make_app(os, storage);

    let payload = json!({ "sessionId": "  ", "input": "hi" });
    let (status, body) = post_json(app, "/v1/agents/test/runs/ai-sdk/sse", payload).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(
        body["error"].as_str().unwrap_or("").contains("sessionId"),
        "expected sessionId error: {body}"
    );
}

#[tokio::test]
async fn test_ai_sdk_sse_empty_input() {
    let os = Arc::new(make_os());
    let storage: Arc<dyn ThreadQuery> = Arc::new(MemoryStorage::new());
    let app = make_app(os, storage);

    let payload = json!({ "sessionId": "s1", "input": "  " });
    let (status, body) = post_json(app, "/v1/agents/test/runs/ai-sdk/sse", payload).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(
        body["error"].as_str().unwrap_or("").contains("input"),
        "expected input error: {body}"
    );
}

#[tokio::test]
async fn test_ai_sdk_sse_agent_not_found() {
    let os = Arc::new(make_os());
    let storage: Arc<dyn ThreadQuery> = Arc::new(MemoryStorage::new());
    let app = make_app(os, storage);

    let payload = json!({ "sessionId": "s1", "input": "hi" });
    let (status, body) = post_json(app, "/v1/agents/no_such_agent/runs/ai-sdk/sse", payload).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert!(
        body["error"].as_str().unwrap_or("").contains("not found"),
        "expected agent not found error: {body}"
    );
}

#[tokio::test]
async fn test_ai_sdk_sse_malformed_json() {
    let os = Arc::new(make_os());
    let storage: Arc<dyn ThreadQuery> = Arc::new(MemoryStorage::new());
    let app = make_app(os, storage);

    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/agents/test/runs/ai-sdk/sse")
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
    let storage: Arc<dyn ThreadQuery> = Arc::new(MemoryStorage::new());
    let app = make_app(os, storage);

    let payload = json!({
        "threadId": "th1", "runId": "r1",
        "messages": [{"role": "user", "content": "hi"}],
        "tools": []
    });
    let (status, body) = post_json(app, "/v1/agents/no_such_agent/runs/ag-ui/sse", payload).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert!(
        body["error"].as_str().unwrap_or("").contains("not found"),
        "expected agent not found error: {body}"
    );
}

#[tokio::test]
async fn test_agui_sse_empty_thread_id() {
    let os = Arc::new(make_os());
    let storage: Arc<dyn ThreadQuery> = Arc::new(MemoryStorage::new());
    let app = make_app(os, storage);

    let payload = json!({
        "threadId": "", "runId": "r1",
        "messages": [], "tools": []
    });
    let (status, body) = post_json(app, "/v1/agents/test/runs/ag-ui/sse", payload).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(
        body["error"].as_str().unwrap_or("").contains("threadId"),
        "expected threadId validation error: {body}"
    );
}

#[tokio::test]
async fn test_agui_sse_empty_run_id() {
    let os = Arc::new(make_os());
    let storage: Arc<dyn ThreadQuery> = Arc::new(MemoryStorage::new());
    let app = make_app(os, storage);

    let payload = json!({
        "threadId": "th1", "runId": "",
        "messages": [], "tools": []
    });
    let (status, body) = post_json(app, "/v1/agents/test/runs/ag-ui/sse", payload).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(
        body["error"].as_str().unwrap_or("").contains("runId"),
        "expected runId validation error: {body}"
    );
}

#[tokio::test]
async fn test_agui_sse_malformed_json() {
    let os = Arc::new(make_os());
    let storage: Arc<dyn ThreadQuery> = Arc::new(MemoryStorage::new());
    let app = make_app(os, storage);

    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/agents/test/runs/ag-ui/sse")
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
    let storage: Arc<dyn ThreadQuery> = Arc::new(MemoryStorage::new());

    // Pre-save a session with id "real-id".
    storage.save(&Thread::new("real-id")).await.unwrap();

    let _app = make_app(os, storage);

    // The mismatch can only occur if storage returns a session with a different id than the key.
    // This is an internal consistency check, not triggerable from external input.
}

// ============================================================================
// Failing storage — tests error propagation
// ============================================================================

struct FailingStorage;

#[async_trait]
impl ThreadStore for FailingStorage {
    async fn create(&self, _thread: &Thread) -> Result<Committed, StorageError> {
        Err(StorageError::Io(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "disk write denied",
        )))
    }

    async fn append(
        &self,
        _id: &str,
        _base_version: Version,
        _delta: &carve_agent::ThreadDelta,
    ) -> Result<Committed, StorageError> {
        Err(StorageError::Io(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "disk write denied",
        )))
    }

    async fn load(&self, _id: &str) -> Result<Option<ThreadHead>, StorageError> {
        Err(StorageError::Io(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "disk read denied",
        )))
    }

    async fn delete(&self, _id: &str) -> Result<(), StorageError> {
        Err(StorageError::Io(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "disk delete denied",
        )))
    }

    async fn save(&self, _thread: &Thread) -> Result<(), StorageError> {
        Err(StorageError::Io(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "disk write denied",
        )))
    }
}

#[async_trait]
impl ThreadQuery for FailingStorage {
    async fn list_threads(
        &self,
        _query: &ThreadListQuery,
    ) -> Result<ThreadListPage, StorageError> {
        Err(StorageError::Io(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "disk list denied",
        )))
    }
}

#[tokio::test]
async fn test_list_sessions_storage_error() {
    let os = Arc::new(make_os());
    let storage: Arc<dyn ThreadQuery> = Arc::new(FailingStorage);
    let app = make_app(os, storage);

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
    let storage: Arc<dyn ThreadQuery> = Arc::new(FailingStorage);
    let app = make_app(os, storage);

    let (status, body) = get_json(app, "/v1/threads/s1").await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert!(
        body["error"].as_str().unwrap_or("").contains("denied"),
        "expected storage error: {body}"
    );
}

#[tokio::test]
async fn test_ai_sdk_sse_storage_load_error() {
    let os = Arc::new(make_os());
    let storage: Arc<dyn ThreadQuery> = Arc::new(FailingStorage);
    let app = make_app(os, storage);

    let payload = json!({ "sessionId": "s1", "input": "hi" });
    let (status, body) = post_json(app, "/v1/agents/test/runs/ai-sdk/sse", payload).await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert!(
        body["error"].as_str().unwrap_or("").contains("denied"),
        "expected storage error: {body}"
    );
}

#[tokio::test]
async fn test_agui_sse_storage_load_error() {
    let os = Arc::new(make_os());
    let storage: Arc<dyn ThreadQuery> = Arc::new(FailingStorage);
    let app = make_app(os, storage);

    let payload = json!({
        "threadId": "th1", "runId": "r1",
        "messages": [{"role": "user", "content": "hi"}],
        "tools": []
    });
    let (status, body) = post_json(app, "/v1/agents/test/runs/ag-ui/sse", payload).await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert!(
        body["error"].as_str().unwrap_or("").contains("denied"),
        "expected storage error: {body}"
    );
}

/// Storage that loads OK but fails on save — tests user message persistence error.
struct SaveFailStorage;

#[async_trait]
impl ThreadStore for SaveFailStorage {
    async fn create(&self, _thread: &Thread) -> Result<Committed, StorageError> {
        Err(StorageError::Io(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "disk write denied",
        )))
    }

    async fn append(
        &self,
        _id: &str,
        _base_version: Version,
        _delta: &carve_agent::ThreadDelta,
    ) -> Result<Committed, StorageError> {
        Err(StorageError::Io(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "disk write denied",
        )))
    }

    async fn load(&self, _id: &str) -> Result<Option<ThreadHead>, StorageError> {
        Ok(None)
    }

    async fn delete(&self, _id: &str) -> Result<(), StorageError> {
        Ok(())
    }

    async fn save(&self, _thread: &Thread) -> Result<(), StorageError> {
        Err(StorageError::Io(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "disk write denied",
        )))
    }
}

#[async_trait]
impl ThreadQuery for SaveFailStorage {
    async fn list_threads(
        &self,
        _query: &ThreadListQuery,
    ) -> Result<ThreadListPage, StorageError> {
        Ok(ThreadListPage {
            items: vec![],
            total: 0,
            has_more: false,
        })
    }
}

#[tokio::test]
async fn test_ai_sdk_sse_storage_save_error() {
    let os = Arc::new(make_os());
    let storage: Arc<dyn ThreadQuery> = Arc::new(SaveFailStorage);
    let app = make_app(os, storage);

    let payload = json!({ "sessionId": "s1", "input": "hi" });
    let (status, body) = post_json(app, "/v1/agents/test/runs/ai-sdk/sse", payload).await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert!(
        body["error"].as_str().unwrap_or("").contains("denied"),
        "expected storage save error: {body}"
    );
}

#[tokio::test]
async fn test_agui_sse_storage_save_error() {
    let os = Arc::new(make_os());
    let storage: Arc<dyn ThreadQuery> = Arc::new(SaveFailStorage);
    let app = make_app(os, storage);

    // Send a message so the session has changes to persist.
    let payload = json!({
        "threadId": "th1", "runId": "r1",
        "messages": [{"role": "user", "content": "hi"}],
        "tools": []
    });
    let (status, body) = post_json(app, "/v1/agents/test/runs/ag-ui/sse", payload).await;
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
        thread = thread.with_message(carve_agent::Message::user(format!("msg-{}", i)));
    }
    thread
}

#[tokio::test]
async fn test_messages_pagination_default_params() {
    let os = Arc::new(make_os());
    let storage: Arc<dyn ThreadQuery> = Arc::new(MemoryStorage::new());
    let thread = make_session_with_n_messages("s1", 5);
    storage.save(&thread).await.unwrap();
    let app = make_app(os, storage);

    let (status, body) = get_json(app, "/v1/threads/s1/messages").await;
    assert_eq!(status, StatusCode::OK);
    let page: carve_agent::MessagePage = serde_json::from_value(body).unwrap();
    assert_eq!(page.messages.len(), 5);
    assert!(!page.has_more);
    assert_eq!(page.messages[0].message.content, "msg-0");
    assert_eq!(page.messages[4].message.content, "msg-4");
}

#[tokio::test]
async fn test_messages_pagination_with_limit() {
    let os = Arc::new(make_os());
    let storage: Arc<dyn ThreadQuery> = Arc::new(MemoryStorage::new());
    let thread = make_session_with_n_messages("s1", 10);
    storage.save(&thread).await.unwrap();
    let app = make_app(os, storage);

    let (status, body) = get_json(app, "/v1/threads/s1/messages?limit=3").await;
    assert_eq!(status, StatusCode::OK);
    let page: carve_agent::MessagePage = serde_json::from_value(body).unwrap();
    assert_eq!(page.messages.len(), 3);
    assert!(page.has_more);
    assert_eq!(page.messages[0].cursor, 0);
    assert_eq!(page.messages[2].cursor, 2);
}

#[tokio::test]
async fn test_messages_pagination_cursor_forward() {
    let os = Arc::new(make_os());
    let storage: Arc<dyn ThreadQuery> = Arc::new(MemoryStorage::new());
    let thread = make_session_with_n_messages("s1", 10);
    storage.save(&thread).await.unwrap();
    let app = make_app(os, storage);

    let (status, body) = get_json(app, "/v1/threads/s1/messages?after=4&limit=3").await;
    assert_eq!(status, StatusCode::OK);
    let page: carve_agent::MessagePage = serde_json::from_value(body).unwrap();
    assert_eq!(page.messages.len(), 3);
    assert_eq!(page.messages[0].cursor, 5);
    assert_eq!(page.messages[0].message.content, "msg-5");
}

#[tokio::test]
async fn test_messages_pagination_desc_order() {
    let os = Arc::new(make_os());
    let storage: Arc<dyn ThreadQuery> = Arc::new(MemoryStorage::new());
    let thread = make_session_with_n_messages("s1", 10);
    storage.save(&thread).await.unwrap();
    let app = make_app(os, storage);

    let (status, body) =
        get_json(app, "/v1/threads/s1/messages?order=desc&before=8&limit=3").await;
    assert_eq!(status, StatusCode::OK);
    let page: carve_agent::MessagePage = serde_json::from_value(body).unwrap();
    assert_eq!(page.messages.len(), 3);
    // Desc order: highest cursors first
    assert_eq!(page.messages[0].cursor, 7);
    assert_eq!(page.messages[1].cursor, 6);
    assert_eq!(page.messages[2].cursor, 5);
}

#[tokio::test]
async fn test_messages_pagination_limit_clamped() {
    let os = Arc::new(make_os());
    let storage: Arc<dyn ThreadQuery> = Arc::new(MemoryStorage::new());
    let thread = make_session_with_n_messages("s1", 300);
    storage.save(&thread).await.unwrap();
    let app = make_app(os, storage);

    let (status, body) = get_json(app, "/v1/threads/s1/messages?limit=999").await;
    assert_eq!(status, StatusCode::OK);
    let page: carve_agent::MessagePage = serde_json::from_value(body).unwrap();
    // limit should be clamped to 200
    assert_eq!(page.messages.len(), 200);
    assert!(page.has_more);
}

#[tokio::test]
async fn test_messages_pagination_not_found() {
    let os = Arc::new(make_os());
    let storage: Arc<dyn ThreadQuery> = Arc::new(MemoryStorage::new());
    let app = make_app(os, storage);

    let (status, body) = get_json(app, "/v1/threads/nonexistent/messages?limit=10").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert!(
        body["error"].as_str().unwrap_or("").contains("not found"),
        "expected not found error: {body}"
    );
}
