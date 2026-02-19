use async_trait::async_trait;
use axum::body::to_bytes;
use axum::http::{Request, StatusCode};
use carve_agentos::contracts::plugin::AgentPlugin;
use carve_agentos::contracts::runtime::phase::{Phase, StepContext};
use carve_agentos::contracts::storage::{
    AgentStateReader, AgentStateStore, AgentStateWriter, VersionPrecondition,
};
use carve_agentos::orchestrator::{AgentDefinition, AgentOs, AgentOsBuilder};
use carve_agentos_server::http::{router, AppState};
use carve_thread_store_adapters::{NatsBufferedThreadWriter, PostgresStore};
use serde_json::json;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;
use testcontainers::runners::AsyncRunner;
use testcontainers::ImageExt;
use testcontainers_modules::nats::Nats;
use testcontainers_modules::postgres::Postgres;
use tower::ServiceExt;

struct SkipInferencePlugin;

#[async_trait]
impl AgentPlugin for SkipInferencePlugin {
    fn id(&self) -> &str {
        "skip_inference_e2e_nats_postgres"
    }

    async fn on_phase(
        &self,
        phase: Phase,
        step: &mut StepContext<'_>,
    ) {
        if phase == Phase::BeforeInference {
            step.skip_inference = true;
        }
    }
}

fn make_os(write_store: Arc<dyn AgentStateStore>) -> AgentOs {
    let def = AgentDefinition {
        id: "test".to_string(),
        plugins: vec![Arc::new(SkipInferencePlugin)],
        ..Default::default()
    };

    AgentOsBuilder::new()
        .with_agent("test", def)
        .with_agent_state_store(write_store)
        .build()
        .expect("failed to build AgentOs")
}

async fn post_json(
    app: axum::Router,
    uri: &str,
    payload: serde_json::Value,
) -> (StatusCode, String) {
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
    let body = to_bytes(resp.into_body(), 4 * 1024 * 1024)
        .await
        .expect("response body should be readable");
    let text = String::from_utf8(body.to_vec()).expect("response body must be utf-8");
    (status, text)
}

async fn start_nats_js() -> Option<(testcontainers::ContainerAsync<Nats>, String)> {
    let container = match Nats::default().with_cmd(["-js"]).start().await {
        Ok(container) => container,
        Err(err) => {
            eprintln!("skipping e2e_nats_postgres: unable to start NATS container ({err})");
            return None;
        }
    };
    let host = container.get_host().await.expect("failed to get host");
    let port = container
        .get_host_port_ipv4(4222)
        .await
        .expect("failed to get port");
    let url = format!("{host}:{port}");
    Some((container, url))
}

async fn start_postgres() -> Option<(testcontainers::ContainerAsync<Postgres>, String)> {
    let container = match Postgres::default().start().await {
        Ok(container) => container,
        Err(err) => {
            eprintln!("skipping e2e_nats_postgres: unable to start Postgres container ({err})");
            return None;
        }
    };
    let host = container.get_host().await.expect("failed to get host");
    let port = container
        .get_host_port_ipv4(5432)
        .await
        .expect("failed to get postgres port");
    let dsn = format!("postgres://postgres:postgres@{host}:{port}/postgres");
    Some((container, dsn))
}

async fn connect_pool_with_retry(dsn: &str) -> sqlx::PgPool {
    let mut last_err = None;
    for _ in 0..40usize {
        match sqlx::PgPool::connect(dsn).await {
            Ok(pool) => return pool,
            Err(err) => {
                last_err = Some(err);
                tokio::time::sleep(Duration::from_millis(150)).await;
            }
        }
    }
    panic!(
        "failed to connect postgres after retries: {}",
        last_err
            .map(|e| e.to_string())
            .unwrap_or_else(|| "unknown error".to_string())
    );
}

fn unique_table(base: &str) -> String {
    format!("{base}_{}", uuid::Uuid::now_v7().simple())
}

struct FlakySaveStore {
    inner: Arc<PostgresStore>,
    fail_saves_remaining: AtomicUsize,
}

impl FlakySaveStore {
    fn new(inner: Arc<PostgresStore>, fail_saves: usize) -> Self {
        Self {
            inner,
            fail_saves_remaining: AtomicUsize::new(fail_saves),
        }
    }
}

#[async_trait]
impl AgentStateWriter for FlakySaveStore {
    async fn create(
        &self,
        thread: &carve_agentos::contracts::state::Thread,
    ) -> Result<
        carve_agentos::contracts::storage::Committed,
        carve_agentos::contracts::storage::AgentStateStoreError,
    > {
        self.inner.create(thread).await
    }

    async fn append(
        &self,
        id: &str,
        delta: &carve_agentos::contracts::ThreadChangeSet,
        precondition: VersionPrecondition,
    ) -> Result<
        carve_agentos::contracts::storage::Committed,
        carve_agentos::contracts::storage::AgentStateStoreError,
    > {
        self.inner.append(id, delta, precondition).await
    }

    async fn delete(
        &self,
        id: &str,
    ) -> Result<(), carve_agentos::contracts::storage::AgentStateStoreError> {
        self.inner.delete(id).await
    }

    async fn save(
        &self,
        thread: &carve_agentos::contracts::state::Thread,
    ) -> Result<(), carve_agentos::contracts::storage::AgentStateStoreError> {
        let remaining = self.fail_saves_remaining.load(Ordering::SeqCst);
        if remaining > 0 {
            self.fail_saves_remaining.fetch_sub(1, Ordering::SeqCst);
            return Err(carve_agentos::contracts::storage::AgentStateStoreError::Io(
                std::io::Error::other("injected save failure"),
            ));
        }
        self.inner.save(thread).await
    }
}

#[async_trait]
impl AgentStateReader for FlakySaveStore {
    async fn load(
        &self,
        id: &str,
    ) -> Result<
        Option<carve_agentos::contracts::storage::AgentStateHead>,
        carve_agentos::contracts::storage::AgentStateStoreError,
    > {
        self.inner.load(id).await
    }

    async fn list_agent_states(
        &self,
        query: &carve_agentos::contracts::storage::AgentStateListQuery,
    ) -> Result<
        carve_agentos::contracts::storage::AgentStateListPage,
        carve_agentos::contracts::storage::AgentStateStoreError,
    > {
        self.inner.list_agent_states(query).await
    }
}

#[tokio::test]
async fn e2e_http_ai_sdk_persists_through_nats_buffered_postgres() {
    let Some((_nats_container, nats_url)) = start_nats_js().await else {
        return;
    };
    let Some((_pg_container, dsn)) = start_postgres().await else {
        return;
    };

    let pool = connect_pool_with_retry(&dsn).await;
    let postgres_store = Arc::new(PostgresStore::with_table(
        pool,
        unique_table("agent_sessions"),
    ));
    postgres_store
        .ensure_table()
        .await
        .expect("postgres table should be created");

    let nats_client = async_nats::connect(&nats_url).await.unwrap();
    let jetstream = async_nats::jetstream::new(nats_client);
    let inner_store: Arc<dyn AgentStateStore> = postgres_store.clone();
    let write_store = Arc::new(
        NatsBufferedThreadWriter::new(inner_store, jetstream)
            .await
            .expect("nats buffered store should initialize"),
    );

    let os = Arc::new(make_os(write_store));
    let read_store: Arc<dyn AgentStateReader> = postgres_store.clone();
    let app = router(AppState { os, read_store });

    let payload = json!({
        "sessionId": "np-e2e-thread",
        "input": "hello np",
        "runId": "np-run-1",
    });
    let (status, body) = post_json(app.clone(), "/v1/ai-sdk/agents/test/runs", payload).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        body.contains(r#""type":"start""#),
        "missing start event: {body}"
    );
    assert!(
        body.contains(r#""type":"finish""#),
        "missing finish event: {body}"
    );

    let mut persisted = None;
    for _ in 0..30usize {
        persisted = postgres_store
            .load_agent_state("np-e2e-thread")
            .await
            .expect("load should not fail");
        if persisted.is_some() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let persisted = persisted.expect("thread should be flushed into postgres");
    assert!(
        persisted.messages.iter().any(|m| m.content == "hello np"),
        "persisted postgres thread should contain input message"
    );
}

#[tokio::test]
async fn e2e_nats_buffered_postgres_recover_replays_pending_deltas() {
    let Some((_nats_container, nats_url)) = start_nats_js().await else {
        return;
    };
    let Some((_pg_container, dsn)) = start_postgres().await else {
        return;
    };

    let pool = connect_pool_with_retry(&dsn).await;
    let postgres_store = Arc::new(PostgresStore::with_table(
        pool,
        unique_table("agent_sessions"),
    ));
    postgres_store
        .ensure_table()
        .await
        .expect("postgres table should be created");

    let nats_client = async_nats::connect(&nats_url).await.unwrap();
    let jetstream = async_nats::jetstream::new(nats_client);
    let inner_store: Arc<dyn AgentStateStore> = postgres_store.clone();
    let storage = NatsBufferedThreadWriter::new(inner_store, jetstream)
        .await
        .expect("nats buffered store should initialize");

    let thread = carve_agentos::contracts::state::Thread::new("np-recover")
        .with_message(carve_agentos::contracts::state::Message::user("hello"));
    storage
        .create(&thread)
        .await
        .expect("create should succeed");

    let delta1 = carve_agentos::contracts::ThreadChangeSet {
        run_id: "np-run-r".to_string(),
        parent_run_id: None,
        reason: carve_agentos::contracts::state::CheckpointReason::AssistantTurnCommitted,
        messages: vec![Arc::new(
            carve_agentos::contracts::state::Message::assistant("mid"),
        )],
        patches: vec![],
        snapshot: None,
    };
    storage
        .append("np-recover", &delta1, VersionPrecondition::Any)
        .await
        .expect("append should succeed");

    let delta2 = carve_agentos::contracts::ThreadChangeSet {
        run_id: "np-run-r".to_string(),
        parent_run_id: None,
        reason: carve_agentos::contracts::state::CheckpointReason::ToolResultsCommitted,
        messages: vec![Arc::new(
            carve_agentos::contracts::state::Message::assistant("tail"),
        )],
        patches: vec![],
        snapshot: None,
    };
    storage
        .append("np-recover", &delta2, VersionPrecondition::Any)
        .await
        .expect("append should succeed");

    let before = postgres_store
        .load_agent_state("np-recover")
        .await
        .expect("load should not fail")
        .expect("thread should exist");
    assert_eq!(before.messages.len(), 1);

    let recovered = storage.recover().await.expect("recover should succeed");
    assert_eq!(recovered, 2);

    let after = postgres_store
        .load_agent_state("np-recover")
        .await
        .expect("load should not fail")
        .expect("thread should exist");
    assert_eq!(after.messages.len(), 3);
    assert_eq!(after.messages[0].content, "hello");
    assert_eq!(after.messages[1].content, "mid");
    assert_eq!(after.messages[2].content, "tail");

    let recovered_again = storage
        .recover()
        .await
        .expect("second recover should succeed");
    assert_eq!(recovered_again, 0, "recover should be idempotent");
}

#[tokio::test]
async fn e2e_http_same_thread_concurrent_runs_preserve_all_user_messages() {
    let Some((_nats_container, nats_url)) = start_nats_js().await else {
        return;
    };
    let Some((_pg_container, dsn)) = start_postgres().await else {
        return;
    };

    let pool = connect_pool_with_retry(&dsn).await;
    let postgres_store = Arc::new(PostgresStore::with_table(
        pool,
        unique_table("agent_sessions"),
    ));
    postgres_store
        .ensure_table()
        .await
        .expect("postgres table should be created");

    let nats_client = async_nats::connect(&nats_url).await.unwrap();
    let jetstream = async_nats::jetstream::new(nats_client);
    let inner_store: Arc<dyn AgentStateStore> = postgres_store.clone();
    let write_store = Arc::new(
        NatsBufferedThreadWriter::new(inner_store, jetstream)
            .await
            .expect("nats buffered store should initialize"),
    );

    let os = Arc::new(make_os(write_store));
    let read_store: Arc<dyn AgentStateReader> = postgres_store.clone();
    let app = router(AppState { os, read_store });

    let total = 8usize;
    let mut handles = Vec::with_capacity(total);
    for i in 0..total {
        let app = app.clone();
        handles.push(tokio::spawn(async move {
            let input = format!("same-thread-input-{i}");
            let payload = json!({
                "sessionId": "np-same-thread",
                "input": input,
                "runId": format!("np-same-run-{i}"),
            });
            let (status, body) = post_json(app, "/v1/ai-sdk/agents/test/runs", payload).await;
            (input, status, body)
        }));
    }

    let mut successful_inputs = Vec::new();
    for handle in handles {
        let (input, status, body) = handle.await.expect("task should complete");
        match status {
            StatusCode::OK => {
                if body.contains(r#""type":"finish""#) {
                    successful_inputs.push(input);
                } else {
                    assert!(
                        body.contains(r#""type":"error""#)
                            || body.contains("checkpoint append failed")
                            || body.contains("not unique on workqueue stream"),
                        "unexpected non-finish successful HTTP response: {body}"
                    );
                }
            }
            StatusCode::INTERNAL_SERVER_ERROR => {
                assert!(
                    body.contains("Version conflict")
                        || body.contains("conflict")
                        || body.contains("already exists"),
                    "unexpected error for concurrent same-thread run: {body}"
                );
            }
            other => {
                panic!("unexpected status for concurrent same-thread run: {other} body={body}")
            }
        }
    }
    assert!(
        !successful_inputs.is_empty(),
        "at least one run should succeed in concurrent same-thread scenario"
    );

    let mut persisted = None;
    for _ in 0..40usize {
        let current = postgres_store
            .load_agent_state("np-same-thread")
            .await
            .expect("load should not fail");
        if let Some(thread) = current {
            let seen = successful_inputs
                .iter()
                .filter(|needle| thread.messages.iter().any(|m| m.content == **needle))
                .count();
            if seen == successful_inputs.len() {
                persisted = Some(thread);
                break;
            }
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    let persisted = persisted.expect("all successful concurrent runs should be persisted");
    for needle in successful_inputs {
        assert!(
            persisted.messages.iter().any(|m| m.content == needle),
            "missing persisted message: {needle}"
        );
    }
}

#[tokio::test]
async fn e2e_nats_buffered_postgres_recover_deduplicates_duplicate_message_ids() {
    let Some((_nats_container, nats_url)) = start_nats_js().await else {
        return;
    };
    let Some((_pg_container, dsn)) = start_postgres().await else {
        return;
    };

    let pool = connect_pool_with_retry(&dsn).await;
    let postgres_store = Arc::new(PostgresStore::with_table(
        pool,
        unique_table("agent_sessions"),
    ));
    postgres_store
        .ensure_table()
        .await
        .expect("postgres table should be created");

    let nats_client = async_nats::connect(&nats_url).await.unwrap();
    let jetstream = async_nats::jetstream::new(nats_client);
    let inner_store: Arc<dyn AgentStateStore> = postgres_store.clone();
    let storage = NatsBufferedThreadWriter::new(inner_store, jetstream)
        .await
        .expect("nats buffered store should initialize");

    let thread = carve_agentos::contracts::state::Thread::new("np-dedup")
        .with_message(carve_agentos::contracts::state::Message::user("seed"));
    storage
        .create(&thread)
        .await
        .expect("create should succeed");

    let duplicate_msg = Arc::new(
        carve_agentos::contracts::state::Message::assistant("dup-mid")
            .with_id("fixed-dup-message-id".to_string()),
    );
    let duplicate_delta = carve_agentos::contracts::ThreadChangeSet {
        run_id: "np-dedup-run".to_string(),
        parent_run_id: None,
        reason: carve_agentos::contracts::state::CheckpointReason::AssistantTurnCommitted,
        messages: vec![duplicate_msg.clone()],
        patches: vec![],
        snapshot: None,
    };
    storage
        .append("np-dedup", &duplicate_delta, VersionPrecondition::Any)
        .await
        .expect("first duplicate append should succeed");
    storage
        .append("np-dedup", &duplicate_delta, VersionPrecondition::Any)
        .await
        .expect("second duplicate append should succeed");

    let recovered = storage.recover().await.expect("recover should succeed");
    assert_eq!(recovered, 2, "both buffered deltas should be consumed");

    let after = postgres_store
        .load_agent_state("np-dedup")
        .await
        .expect("load should not fail")
        .expect("thread should exist");
    let dup_count = after
        .messages
        .iter()
        .filter(|m| m.content == "dup-mid")
        .count();
    assert_eq!(
        dup_count, 1,
        "duplicate message ids should be deduplicated during materialize/save"
    );
}

#[tokio::test]
async fn e2e_nats_buffered_postgres_flush_retry_after_transient_save_failure() {
    let Some((_nats_container, nats_url)) = start_nats_js().await else {
        return;
    };
    let Some((_pg_container, dsn)) = start_postgres().await else {
        return;
    };

    let pool = connect_pool_with_retry(&dsn).await;
    let postgres_store = Arc::new(PostgresStore::with_table(
        pool,
        unique_table("agent_sessions"),
    ));
    postgres_store
        .ensure_table()
        .await
        .expect("postgres table should be created");

    let flaky_inner = Arc::new(FlakySaveStore::new(postgres_store.clone(), 1));
    let nats_client = async_nats::connect(&nats_url).await.unwrap();
    let jetstream = async_nats::jetstream::new(nats_client);
    let inner_store: Arc<dyn AgentStateStore> = flaky_inner;
    let storage = NatsBufferedThreadWriter::new(inner_store, jetstream)
        .await
        .expect("nats buffered store should initialize");

    let thread = carve_agentos::contracts::state::Thread::new("np-flaky")
        .with_message(carve_agentos::contracts::state::Message::user("seed"));
    storage
        .create(&thread)
        .await
        .expect("create should succeed");

    let mid = carve_agentos::contracts::ThreadChangeSet {
        run_id: "np-flaky-run".to_string(),
        parent_run_id: None,
        reason: carve_agentos::contracts::state::CheckpointReason::AssistantTurnCommitted,
        messages: vec![Arc::new(
            carve_agentos::contracts::state::Message::assistant("mid"),
        )],
        patches: vec![],
        snapshot: None,
    };
    storage
        .append("np-flaky", &mid, VersionPrecondition::Any)
        .await
        .expect("buffer append should succeed");

    let run_finished = carve_agentos::contracts::ThreadChangeSet {
        run_id: "np-flaky-run".to_string(),
        parent_run_id: None,
        reason: carve_agentos::contracts::state::CheckpointReason::RunFinished,
        messages: vec![Arc::new(
            carve_agentos::contracts::state::Message::assistant("tail"),
        )],
        patches: vec![],
        snapshot: None,
    };
    let flush_err = storage
        .append("np-flaky", &run_finished, VersionPrecondition::Any)
        .await
        .expect_err("first flush should fail due to injected save error");
    assert!(
        flush_err.to_string().contains("injected save failure"),
        "unexpected first flush error: {flush_err}"
    );

    let before_recover = postgres_store
        .load_agent_state("np-flaky")
        .await
        .expect("load should not fail")
        .expect("thread should exist");
    assert_eq!(
        before_recover.messages.len(),
        1,
        "failed flush must not partially commit buffered deltas"
    );

    let recovered = storage.recover().await.expect("recover should succeed");
    assert_eq!(recovered, 2, "recover should replay both pending deltas");

    let after_recover = postgres_store
        .load_agent_state("np-flaky")
        .await
        .expect("load should not fail")
        .expect("thread should exist");
    assert_eq!(after_recover.messages.len(), 3);
    assert_eq!(after_recover.messages[0].content, "seed");
    assert_eq!(after_recover.messages[1].content, "mid");
    assert_eq!(after_recover.messages[2].content, "tail");

    let recovered_again = storage
        .recover()
        .await
        .expect("second recover should succeed");
    assert_eq!(
        recovered_again, 0,
        "recover should be idempotent after success"
    );
}
