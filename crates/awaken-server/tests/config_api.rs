//! Config management API integration tests.
//!
//! Verifies the /v1/config/:namespace CRUD routes and that agents
//! created via the API can be resolved and executed by the runtime.

use std::sync::Arc;

use async_trait::async_trait;
use awaken_contract::contract::config_store::ConfigStore;
use awaken_contract::contract::executor::{InferenceExecutionError, InferenceRequest};
use awaken_contract::contract::inference::{StopReason, StreamResult, TokenUsage};
use awaken_contract::registry_spec::{AgentSpec, ModelSpec};
use awaken_runtime::builder::AgentRuntimeBuilder;
use awaken_runtime::registry::store_backed::StoreBackedAgentRegistry;
use awaken_runtime::registry::traits::AgentSpecRegistry;
use awaken_server::app::{AppState, ServerConfig};
use awaken_server::routes::build_router;
use awaken_stores::memory::InMemoryStore;
use axum::body::to_bytes;
use axum::http::{Request, StatusCode};
use serde_json::{Value, json};
use tower::ServiceExt;

// ── Mock executor ──

struct ImmediateExecutor;

#[async_trait]
impl awaken_contract::contract::executor::LlmExecutor for ImmediateExecutor {
    async fn execute(
        &self,
        _request: InferenceRequest,
    ) -> Result<StreamResult, InferenceExecutionError> {
        Ok(StreamResult {
            content: vec![],
            tool_calls: vec![],
            usage: Some(TokenUsage::default()),
            stop_reason: Some(StopReason::EndTurn),
            has_incomplete_tool_calls: false,
        })
    }

    fn name(&self) -> &str {
        "immediate"
    }
}

// ── In-memory ConfigStore for tests ──

#[derive(Debug, Default)]
struct MemoryConfigStore {
    data: tokio::sync::RwLock<
        std::collections::HashMap<String, std::collections::HashMap<String, Value>>,
    >,
}

#[async_trait]
impl ConfigStore for MemoryConfigStore {
    async fn get(
        &self,
        namespace: &str,
        id: &str,
    ) -> Result<Option<Value>, awaken_contract::contract::storage::StorageError> {
        Ok(self
            .data
            .read()
            .await
            .get(namespace)
            .and_then(|ns| ns.get(id))
            .cloned())
    }

    async fn list(
        &self,
        namespace: &str,
        offset: usize,
        limit: usize,
    ) -> Result<Vec<(String, Value)>, awaken_contract::contract::storage::StorageError> {
        let data = self.data.read().await;
        let Some(ns) = data.get(namespace) else {
            return Ok(Vec::new());
        };
        let mut entries: Vec<_> = ns.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
        entries.sort_by(|a, b| a.0.cmp(&b.0));
        Ok(entries.into_iter().skip(offset).take(limit).collect())
    }

    async fn put(
        &self,
        namespace: &str,
        id: &str,
        value: &Value,
    ) -> Result<(), awaken_contract::contract::storage::StorageError> {
        self.data
            .write()
            .await
            .entry(namespace.to_string())
            .or_default()
            .insert(id.to_string(), value.clone());
        Ok(())
    }

    async fn delete(
        &self,
        namespace: &str,
        id: &str,
    ) -> Result<(), awaken_contract::contract::storage::StorageError> {
        if let Some(ns) = self.data.write().await.get_mut(namespace) {
            ns.remove(id);
        }
        Ok(())
    }
}

// ── Test app builder ──

struct TestApp {
    router: axum::Router,
    config_store: Arc<MemoryConfigStore>,
}

fn make_test_app() -> TestApp {
    let thread_store = Arc::new(InMemoryStore::new());
    let config_store = Arc::new(MemoryConfigStore::default());

    let runtime = Arc::new(
        AgentRuntimeBuilder::new()
            .with_model(
                "test-model",
                ModelSpec {
                    id: String::new(),
                    provider: "mock".into(),
                    model: "mock-model".into(),
                },
            )
            .with_provider("mock", Arc::new(ImmediateExecutor))
            .with_agent_spec(AgentSpec {
                id: "builtin-agent".into(),
                model: "test-model".into(),
                system_prompt: "builtin".into(),
                max_rounds: 0,
                ..Default::default()
            })
            .with_thread_run_store(thread_store.clone())
            .build()
            .expect("build runtime"),
    );
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
        thread_store,
        runtime.resolver_arc(),
        ServerConfig::default(),
    )
    .with_config_store(config_store.clone() as Arc<dyn ConfigStore>);

    TestApp {
        router: build_router().with_state(state),
        config_store,
    }
}

// ── HTTP helpers ──

async fn get_json(app: axum::Router, uri: &str) -> (StatusCode, Value) {
    let resp = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(uri)
                .body(axum::body::Body::empty())
                .expect("request"),
        )
        .await
        .expect("handle");
    let status = resp.status();
    let body = to_bytes(resp.into_body(), 1024 * 1024).await.expect("body");
    let json: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
    (status, json)
}

async fn post_json(app: axum::Router, uri: &str, payload: Value) -> (StatusCode, Value) {
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(uri)
                .header("content-type", "application/json")
                .body(axum::body::Body::from(payload.to_string()))
                .expect("request"),
        )
        .await
        .expect("handle");
    let status = resp.status();
    let body = to_bytes(resp.into_body(), 1024 * 1024).await.expect("body");
    let json: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
    (status, json)
}

async fn put_json(app: axum::Router, uri: &str, payload: Value) -> (StatusCode, Value) {
    let resp = app
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri(uri)
                .header("content-type", "application/json")
                .body(axum::body::Body::from(payload.to_string()))
                .expect("request"),
        )
        .await
        .expect("handle");
    let status = resp.status();
    let body = to_bytes(resp.into_body(), 1024 * 1024).await.expect("body");
    let json: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
    (status, json)
}

async fn delete_req(app: axum::Router, uri: &str) -> StatusCode {
    let resp = app
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(uri)
                .body(axum::body::Body::empty())
                .expect("request"),
        )
        .await
        .expect("handle");
    resp.status()
}

// ========================================================================
// Config CRUD tests
// ========================================================================

#[tokio::test]
async fn create_and_get_agent_via_api() {
    let test = make_test_app();

    let agent = json!({
        "id": "api-agent",
        "model": "test-model",
        "system_prompt": "Created via API",
        "plugin_ids": ["permission"],
        "sections": {
            "permission": {"default_behavior": "ask"}
        }
    });

    // Create
    let (status, body) = post_json(test.router.clone(), "/v1/config/agents", agent.clone()).await;
    assert_eq!(status, StatusCode::CREATED, "create failed: {body}");
    assert_eq!(body["id"], "api-agent");

    // Get
    let (status, body) = get_json(test.router.clone(), "/v1/config/agents/api-agent").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["id"], "api-agent");
    assert_eq!(body["model"], "test-model");
    assert_eq!(body["system_prompt"], "Created via API");
    assert_eq!(body["sections"]["permission"]["default_behavior"], "ask");
}

#[tokio::test]
async fn list_agents_via_api() {
    let test = make_test_app();

    for name in ["alpha", "bravo", "charlie"] {
        post_json(
            test.router.clone(),
            "/v1/config/agents",
            json!({"id": name, "model": "m", "system_prompt": "sp"}),
        )
        .await;
    }

    let (status, body) = get_json(test.router, "/v1/config/agents").await;
    assert_eq!(status, StatusCode::OK);
    let items = body["items"].as_array().unwrap();
    assert_eq!(items.len(), 3);
    // Sorted by ID
    assert_eq!(items[0]["id"], "alpha");
    assert_eq!(items[1]["id"], "bravo");
    assert_eq!(items[2]["id"], "charlie");
}

#[tokio::test]
async fn update_agent_via_api() {
    let test = make_test_app();

    post_json(
        test.router.clone(),
        "/v1/config/agents",
        json!({"id": "a1", "model": "m", "system_prompt": "v1"}),
    )
    .await;

    let (status, _) = put_json(
        test.router.clone(),
        "/v1/config/agents/a1",
        json!({"id": "a1", "model": "m", "system_prompt": "v2", "max_rounds": 32}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (_, body) = get_json(test.router, "/v1/config/agents/a1").await;
    assert_eq!(body["system_prompt"], "v2");
    assert_eq!(body["max_rounds"], 32);
}

#[tokio::test]
async fn delete_agent_via_api() {
    let test = make_test_app();

    post_json(
        test.router.clone(),
        "/v1/config/agents",
        json!({"id": "del-me", "model": "m", "system_prompt": "sp"}),
    )
    .await;

    let status = delete_req(test.router.clone(), "/v1/config/agents/del-me").await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let (status, _) = get_json(test.router, "/v1/config/agents/del-me").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn create_duplicate_agent_fails() {
    let test = make_test_app();
    let agent = json!({"id": "dup", "model": "m", "system_prompt": "sp"});

    let (status, _) = post_json(test.router.clone(), "/v1/config/agents", agent.clone()).await;
    assert_eq!(status, StatusCode::CREATED);

    let (status, body) = post_json(test.router, "/v1/config/agents", agent).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body["error"].as_str().unwrap().contains("already exists"));
}

#[tokio::test]
async fn get_nonexistent_agent_returns_404() {
    let test = make_test_app();
    let (status, _) = get_json(test.router, "/v1/config/agents/nope").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

// ========================================================================
// Multi-namespace tests via API
// ========================================================================

#[tokio::test]
async fn model_crud_via_api() {
    let test = make_test_app();

    let model = json!({"id": "opus", "provider": "anthropic", "model": "claude-opus-4-6"});
    let (status, _) = post_json(test.router.clone(), "/v1/config/models", model).await;
    assert_eq!(status, StatusCode::CREATED);

    let (status, body) = get_json(test.router, "/v1/config/models/opus").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["provider"], "anthropic");
    assert_eq!(body["model"], "claude-opus-4-6");
}

// ========================================================================
// Capabilities endpoint
// ========================================================================

#[tokio::test]
async fn capabilities_lists_agents_and_namespaces() {
    let test = make_test_app();
    let (status, body) = get_json(test.router, "/v1/capabilities").await;
    assert_eq!(status, StatusCode::OK);

    // builtin-agent registered in runtime
    let agents = body["agents"].as_array().unwrap();
    assert!(agents.iter().any(|a| a == "builtin-agent"));

    // Namespaces are objects with "namespace" and "schema" fields
    let namespaces = body["namespaces"].as_array().unwrap();
    assert!(namespaces.iter().any(|n| n["namespace"] == "agents"));
    assert!(namespaces.iter().any(|n| n["namespace"] == "models"));

    // Each namespace has a JSON Schema
    let agents_ns = namespaces
        .iter()
        .find(|n| n["namespace"] == "agents")
        .unwrap();
    assert!(agents_ns["schema"]["properties"]["id"].is_object());

    // Tools and plugins are arrays (may be empty in test)
    assert!(body["tools"].is_array());
    assert!(body["plugins"].is_array());
}

// ========================================================================
// Convenience aliases
// ========================================================================

#[tokio::test]
async fn convenience_agents_alias_works() {
    let test = make_test_app();

    post_json(
        test.router.clone(),
        "/v1/config/agents",
        json!({"id": "via-alias", "model": "m", "system_prompt": "sp"}),
    )
    .await;

    // /v1/agents/:id is an alias for /v1/config/agents/:id
    let (status, body) = get_json(test.router, "/v1/agents/via-alias").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["id"], "via-alias");
}

// ========================================================================
// Agent created via API can be resolved by StoreBackedAgentRegistry
// ========================================================================

#[tokio::test]
async fn agent_from_config_store_resolves_in_registry() {
    let test = make_test_app();

    // Create agent via config API
    let agent_spec = AgentSpec {
        id: "dynamic-agent".into(),
        model: "test-model".into(),
        system_prompt: "I am dynamic.".into(),
        max_rounds: 5,
        ..Default::default()
    };
    test.config_store
        .put(
            "agents",
            "dynamic-agent",
            &serde_json::to_value(&agent_spec).unwrap(),
        )
        .await
        .unwrap();

    // Create StoreBackedAgentRegistry from the same store
    let registry = StoreBackedAgentRegistry::new(test.config_store.clone() as Arc<dyn ConfigStore>);
    registry.refresh().await.unwrap();

    // Verify the agent is resolvable
    let loaded = registry.get_agent("dynamic-agent").unwrap();
    assert_eq!(loaded.system_prompt, "I am dynamic.");
    assert_eq!(loaded.max_rounds, 5);
}

#[tokio::test]
async fn agent_with_plugin_sections_survives_api_roundtrip() {
    let test = make_test_app();

    let spec = json!({
        "id": "full-agent",
        "model": "test-model",
        "system_prompt": "Full featured",
        "plugin_ids": ["permission", "deferred-tools"],
        "sections": {
            "permission": {"default_behavior": "deny", "rules": [
                {"tool": "Bash", "behavior": "ask"}
            ]},
            "deferred_tools": {"enabled": true}
        },
        "allowed_tools": ["Bash", "Read"],
        "delegates": ["reviewer"]
    });

    let (status, _) = post_json(test.router.clone(), "/v1/config/agents", spec).await;
    assert_eq!(status, StatusCode::CREATED);

    // Reload through StoreBackedAgentRegistry
    let registry = StoreBackedAgentRegistry::new(test.config_store.clone() as Arc<dyn ConfigStore>);
    registry.refresh().await.unwrap();

    let loaded = registry.get_agent("full-agent").unwrap();
    assert_eq!(loaded.plugin_ids, vec!["permission", "deferred-tools"]);
    assert_eq!(loaded.sections["permission"]["default_behavior"], "deny");
    assert_eq!(loaded.sections["deferred_tools"]["enabled"], true);
    assert_eq!(
        loaded.allowed_tools,
        Some(vec!["Bash".into(), "Read".into()])
    );
    assert_eq!(loaded.delegates, vec!["reviewer"]);
}
