//! Integration tests for FileConfigStore + ConfigRegistry.

#![cfg(feature = "file")]

use std::collections::HashMap;
use std::sync::Arc;

use awaken_contract::contract::config_store::{ConfigNamespace, ConfigRegistry, ConfigStore};
use awaken_contract::registry_spec::{AgentSpec, McpServerSpec, ModelSpec, ProviderSpec};
use awaken_stores::FileConfigStore;
use serde_json::json;
use tempfile::TempDir;

// ── Namespace definitions (using formal Spec types from awaken-contract) ──

struct AgentNamespace;
impl ConfigNamespace for AgentNamespace {
    const NAMESPACE: &'static str = "agents";
    type Value = AgentSpec;
    fn id(value: &AgentSpec) -> &str {
        &value.id
    }
}

struct ModelNamespace;
impl ConfigNamespace for ModelNamespace {
    const NAMESPACE: &'static str = "models";
    type Value = ModelSpec;
    fn id(value: &ModelSpec) -> &str {
        &value.id
    }
}

struct ProviderNamespace;
impl ConfigNamespace for ProviderNamespace {
    const NAMESPACE: &'static str = "providers";
    type Value = ProviderSpec;
    fn id(value: &ProviderSpec) -> &str {
        &value.id
    }
}

struct McpServerNamespace;
impl ConfigNamespace for McpServerNamespace {
    const NAMESPACE: &'static str = "mcp-servers";
    type Value = McpServerSpec;
    fn id(value: &McpServerSpec) -> &str {
        &value.id
    }
}

// ── Helpers ──────────────────────────────────────────────────────────

fn make_store(td: &TempDir) -> Arc<FileConfigStore> {
    Arc::new(FileConfigStore::new(td.path()))
}

fn sample_agent(id: &str, model: &str) -> AgentSpec {
    AgentSpec {
        id: id.into(),
        model: model.into(),
        system_prompt: format!("You are {id}."),
        ..Default::default()
    }
}

fn sample_agent_with_plugins(id: &str) -> AgentSpec {
    AgentSpec {
        id: id.into(),
        model: "claude-opus".into(),
        system_prompt: "You code.".into(),
        plugin_ids: vec!["permission".into(), "deferred-tools".into()],
        sections: {
            let mut s = HashMap::new();
            s.insert(
                "permission".into(),
                json!({"default_behavior": "ask", "rules": []}),
            );
            s.insert(
                "deferred_tools".into(),
                json!({"enabled": true, "default_mode": "deferred"}),
            );
            s
        },
        allowed_tools: Some(vec!["Bash".into(), "Read".into()]),
        delegates: vec!["reviewer".into()],
        ..Default::default()
    }
}

// ========================================================================
// AgentSpec CRUD through ConfigRegistry
// ========================================================================

#[tokio::test]
async fn agent_crud_lifecycle() {
    let td = TempDir::new().unwrap();
    let reg = ConfigRegistry::<AgentNamespace>::new(make_store(&td));

    // Create
    let spec = sample_agent("coder", "claude-opus");
    reg.put(&spec).await.unwrap();

    // Read
    let loaded = reg.get("coder").await.unwrap().unwrap();
    assert_eq!(loaded.id, "coder");
    assert_eq!(loaded.model, "claude-opus");
    assert_eq!(loaded.system_prompt, "You are coder.");

    // Update
    let mut updated = loaded;
    updated.system_prompt = "Updated prompt.".into();
    updated.max_rounds = 32;
    reg.put(&updated).await.unwrap();

    let reloaded = reg.get("coder").await.unwrap().unwrap();
    assert_eq!(reloaded.system_prompt, "Updated prompt.");
    assert_eq!(reloaded.max_rounds, 32);

    // Delete
    reg.delete("coder").await.unwrap();
    assert!(reg.get("coder").await.unwrap().is_none());
}

#[tokio::test]
async fn agent_with_plugin_sections_roundtrip() {
    let td = TempDir::new().unwrap();
    let reg = ConfigRegistry::<AgentNamespace>::new(make_store(&td));

    let spec = sample_agent_with_plugins("coder");
    reg.put(&spec).await.unwrap();

    let loaded = reg.get("coder").await.unwrap().unwrap();
    assert_eq!(loaded.plugin_ids, vec!["permission", "deferred-tools"]);
    assert_eq!(loaded.sections["permission"]["default_behavior"], "ask");
    assert_eq!(loaded.sections["deferred_tools"]["enabled"], true);
    assert_eq!(
        loaded.allowed_tools,
        Some(vec!["Bash".into(), "Read".into()])
    );
    assert_eq!(loaded.delegates, vec!["reviewer"]);
}

#[tokio::test]
async fn agent_inactive_plugin_config_preserved_on_update() {
    let td = TempDir::new().unwrap();
    let reg = ConfigRegistry::<AgentNamespace>::new(make_store(&td));

    // Save agent with two plugin configs
    let spec = sample_agent_with_plugins("coder");
    reg.put(&spec).await.unwrap();

    // Update: remove "deferred-tools" from plugin_ids but keep its section
    let mut updated = reg.get("coder").await.unwrap().unwrap();
    updated.plugin_ids = vec!["permission".into()]; // deferred-tools removed
    // sections NOT modified — deferred_tools config stays
    reg.put(&updated).await.unwrap();

    let reloaded = reg.get("coder").await.unwrap().unwrap();
    assert_eq!(reloaded.plugin_ids, vec!["permission"]);
    // Config for inactive plugin is preserved
    assert!(
        reloaded.sections.contains_key("deferred_tools"),
        "inactive plugin config must survive update"
    );
    assert_eq!(reloaded.sections["deferred_tools"]["enabled"], true);
}

#[tokio::test]
async fn agent_list_with_pagination() {
    let td = TempDir::new().unwrap();
    let reg = ConfigRegistry::<AgentNamespace>::new(make_store(&td));

    for name in ["delta", "alpha", "charlie", "bravo"] {
        reg.put(&sample_agent(name, "m")).await.unwrap();
    }

    // Full list (sorted)
    let all = reg.list(0, 100).await.unwrap();
    let ids: Vec<&str> = all.iter().map(|s| s.id.as_str()).collect();
    assert_eq!(ids, vec!["alpha", "bravo", "charlie", "delta"]);

    // Paginated
    let page = reg.list(1, 2).await.unwrap();
    let ids: Vec<&str> = page.iter().map(|s| s.id.as_str()).collect();
    assert_eq!(ids, vec!["bravo", "charlie"]);
}

// ========================================================================
// Multi-namespace isolation
// ========================================================================

#[tokio::test]
async fn namespaces_fully_isolated() {
    let td = TempDir::new().unwrap();
    let store = make_store(&td);

    let agents = ConfigRegistry::<AgentNamespace>::new(store.clone());
    let models = ConfigRegistry::<ModelNamespace>::new(store.clone());
    let providers = ConfigRegistry::<ProviderNamespace>::new(store);

    // All use the same underlying store
    agents.put(&sample_agent("coder", "claude")).await.unwrap();
    models
        .put(&ModelSpec {
            id: "claude".into(),
            provider: "anthropic".into(),
            model: "claude-opus-4-6".into(),
        })
        .await
        .unwrap();
    providers
        .put(&ProviderSpec {
            id: "anthropic".into(),
            adapter: "anthropic".into(),
            api_key: Some("sk-test-key".into()),
            ..Default::default()
        })
        .await
        .unwrap();

    // Each namespace has exactly 1 entry
    assert_eq!(agents.list(0, 100).await.unwrap().len(), 1);
    assert_eq!(models.list(0, 100).await.unwrap().len(), 1);
    assert_eq!(providers.list(0, 100).await.unwrap().len(), 1);

    // Delete from one namespace doesn't affect others
    agents.delete("coder").await.unwrap();
    assert!(agents.get("coder").await.unwrap().is_none());
    assert!(models.get("claude").await.unwrap().is_some());
    assert!(providers.get("anthropic").await.unwrap().is_some());
}

// ========================================================================
// ModelConfig + ProviderConfig CRUD
// ========================================================================

#[tokio::test]
async fn model_config_crud() {
    let td = TempDir::new().unwrap();
    let reg = ConfigRegistry::<ModelNamespace>::new(make_store(&td));

    let config = ModelSpec {
        id: "claude-opus".into(),
        provider: "anthropic".into(),
        model: "claude-opus-4-6".into(),
    };
    reg.put(&config).await.unwrap();

    let loaded = reg.get("claude-opus").await.unwrap().unwrap();
    assert_eq!(loaded.provider, "anthropic");
    assert_eq!(loaded.model, "claude-opus-4-6");

    // Update model name
    let updated = ModelSpec {
        model: "claude-opus-4-0-20250514".into(),
        ..loaded
    };
    reg.put(&updated).await.unwrap();
    let reloaded = reg.get("claude-opus").await.unwrap().unwrap();
    assert_eq!(reloaded.model, "claude-opus-4-0-20250514");
}

#[tokio::test]
async fn provider_config_with_api_key_roundtrip() {
    let td = TempDir::new().unwrap();
    let reg = ConfigRegistry::<ProviderNamespace>::new(make_store(&td));

    let config = ProviderSpec {
        id: "openai".into(),
        adapter: "openai".into(),
        api_key: Some("sk-proj-abc123".into()),
        base_url: Some("https://proxy.example.com/v1".into()),
        ..Default::default()
    };
    reg.put(&config).await.unwrap();

    let loaded = reg.get("openai").await.unwrap().unwrap();
    assert_eq!(loaded.adapter, "openai");
    assert_eq!(loaded.api_key.as_deref(), Some("sk-proj-abc123"));
    assert_eq!(
        loaded.base_url.as_deref(),
        Some("https://proxy.example.com/v1")
    );
}

#[tokio::test]
async fn provider_config_without_optional_fields() {
    let td = TempDir::new().unwrap();
    let reg = ConfigRegistry::<ProviderNamespace>::new(make_store(&td));

    let config = ProviderSpec {
        id: "ollama".into(),
        adapter: "ollama".into(),
        ..Default::default()
    };
    reg.put(&config).await.unwrap();

    let loaded = reg.get("ollama").await.unwrap().unwrap();
    assert!(loaded.api_key.is_none());
    assert!(loaded.base_url.is_none());
}

// ========================================================================
// Cross-references (agent → model → provider)
// ========================================================================

#[tokio::test]
async fn agent_references_model_references_provider() {
    let td = TempDir::new().unwrap();
    let store = make_store(&td);

    let providers = ConfigRegistry::<ProviderNamespace>::new(store.clone());
    let models = ConfigRegistry::<ModelNamespace>::new(store.clone());
    let agents = ConfigRegistry::<AgentNamespace>::new(store);

    // Create provider
    providers
        .put(&ProviderSpec {
            id: "anthropic".into(),
            adapter: "anthropic".into(),
            api_key: Some("sk-test".into()),
            ..Default::default()
        })
        .await
        .unwrap();

    // Create model referencing provider
    models
        .put(&ModelSpec {
            id: "opus".into(),
            provider: "anthropic".into(),
            model: "claude-opus-4-6".into(),
        })
        .await
        .unwrap();

    // Create agent referencing model
    agents.put(&sample_agent("coder", "opus")).await.unwrap();

    // Verify reference chain
    let agent = agents.get("coder").await.unwrap().unwrap();
    assert_eq!(agent.model, "opus");

    let model = models.get(&agent.model).await.unwrap().unwrap();
    assert_eq!(model.provider, "anthropic");

    let provider = providers.get(&model.provider).await.unwrap().unwrap();
    assert_eq!(provider.adapter, "anthropic");
    assert!(provider.api_key.is_some());
}

// ========================================================================
// Raw ConfigStore with unknown namespace (forward compatible)
// ========================================================================

#[tokio::test]
async fn raw_store_accepts_any_namespace() {
    let td = TempDir::new().unwrap();
    let store = make_store(&td);

    // Future namespace not yet defined as ConfigNamespace
    store
        .put("workflows", "wf-1", &json!({"id": "wf-1", "steps": []}))
        .await
        .unwrap();

    let loaded = store.get("workflows", "wf-1").await.unwrap().unwrap();
    assert_eq!(loaded["id"], "wf-1");
}

// ========================================================================
// Edge cases
// ========================================================================

#[tokio::test]
async fn agent_spec_defaults_preserved() {
    let td = TempDir::new().unwrap();
    let reg = ConfigRegistry::<AgentNamespace>::new(make_store(&td));

    // Minimal agent
    let spec = AgentSpec {
        id: "minimal".into(),
        model: "m".into(),
        system_prompt: "sp".into(),
        ..Default::default()
    };
    reg.put(&spec).await.unwrap();

    let loaded = reg.get("minimal").await.unwrap().unwrap();
    assert_eq!(loaded.max_rounds, 16); // default
    assert_eq!(loaded.max_continuation_retries, 2); // default
    assert!(loaded.plugin_ids.is_empty());
    assert!(loaded.sections.is_empty());
    assert!(loaded.allowed_tools.is_none());
    assert!(loaded.delegates.is_empty());
}

#[tokio::test]
async fn concurrent_writes_to_different_namespaces() {
    let td = TempDir::new().unwrap();
    let store = make_store(&td);

    let agents = ConfigRegistry::<AgentNamespace>::new(store.clone());
    let models = ConfigRegistry::<ModelNamespace>::new(store);

    // Concurrent writes
    let agent_val = sample_agent("a1", "m1");
    let model_val = ModelSpec {
        id: "m1".into(),
        provider: "p".into(),
        model: "model-v1".into(),
    };
    let (r1, r2) = tokio::join!(agents.put(&agent_val), models.put(&model_val));
    r1.unwrap();
    r2.unwrap();

    assert!(agents.get("a1").await.unwrap().is_some());
    assert!(models.get("m1").await.unwrap().is_some());
}
