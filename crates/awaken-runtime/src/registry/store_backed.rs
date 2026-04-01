//! Registry implementations backed by a [`ConfigStore`].
//!
//! Provides [`StoreBackedAgentRegistry`] and [`StoreBackedModelRegistry`] that
//! bridge async [`ConfigStore`] persistence to the synchronous [`AgentSpecRegistry`]
//! and [`ModelRegistry`] traits via an in-memory cache.
//!
//! See ADR-0021.

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use awaken_contract::contract::config_store::{ConfigNamespace, ConfigStore};
use awaken_contract::contract::storage::StorageError;
use awaken_contract::registry_spec::{AgentSpec, ModelSpec};

use super::traits::{AgentSpecRegistry, ModelRegistry};

// ---------------------------------------------------------------------------
// ConfigNamespace impls — these bind Spec types to namespace strings
// ---------------------------------------------------------------------------

/// Namespace for agent specs.
pub struct AgentNamespace;

impl ConfigNamespace for AgentNamespace {
    const NAMESPACE: &'static str = "agents";
    type Value = AgentSpec;
    fn id(value: &AgentSpec) -> &str {
        &value.id
    }
}

/// Namespace for model specs.
pub struct ModelNamespace;

impl ConfigNamespace for ModelNamespace {
    const NAMESPACE: &'static str = "models";
    type Value = ModelSpec;
    fn id(value: &ModelSpec) -> &str {
        &value.id
    }
}

// ---------------------------------------------------------------------------
// StoreBackedAgentRegistry
// ---------------------------------------------------------------------------

/// [`AgentSpecRegistry`] backed by [`ConfigStore`] with an in-memory cache.
///
/// On startup, call [`refresh()`](Self::refresh) to load all agents from the
/// store into the cache. After mutations via the management API, call
/// `refresh()` again to reload.
pub struct StoreBackedAgentRegistry {
    store: Arc<dyn ConfigStore>,
    cache: RwLock<HashMap<String, AgentSpec>>,
}

impl StoreBackedAgentRegistry {
    /// Create a new registry backed by the given store. Cache starts empty —
    /// call [`refresh()`](Self::refresh) to populate.
    pub fn new(store: Arc<dyn ConfigStore>) -> Self {
        Self {
            store,
            cache: RwLock::new(HashMap::new()),
        }
    }

    /// Reload the cache from the store.
    pub async fn refresh(&self) -> Result<(), StorageError> {
        let entries = self
            .store
            .list(AgentNamespace::NAMESPACE, 0, usize::MAX)
            .await?;
        let mut map = HashMap::with_capacity(entries.len());
        for (id, value) in entries {
            let spec: AgentSpec = serde_json::from_value(value)
                .map_err(|e| StorageError::Serialization(e.to_string()))?;
            map.insert(id, spec);
        }
        *self.cache.write().unwrap() = map;
        Ok(())
    }

    /// Invalidate a single entry (e.g. after delete).
    pub fn invalidate(&self, id: &str) {
        self.cache.write().unwrap().remove(id);
    }

    /// Update a single cached entry (e.g. after put).
    pub fn update_cached(&self, spec: AgentSpec) {
        self.cache.write().unwrap().insert(spec.id.clone(), spec);
    }
}

impl AgentSpecRegistry for StoreBackedAgentRegistry {
    fn get_agent(&self, id: &str) -> Option<AgentSpec> {
        self.cache.read().unwrap().get(id).cloned()
    }

    fn agent_ids(&self) -> Vec<String> {
        self.cache.read().unwrap().keys().cloned().collect()
    }
}

// ---------------------------------------------------------------------------
// StoreBackedModelRegistry
// ---------------------------------------------------------------------------

/// [`ModelRegistry`] backed by [`ConfigStore`] with an in-memory cache.
pub struct StoreBackedModelRegistry {
    store: Arc<dyn ConfigStore>,
    cache: RwLock<HashMap<String, ModelSpec>>,
}

impl StoreBackedModelRegistry {
    pub fn new(store: Arc<dyn ConfigStore>) -> Self {
        Self {
            store,
            cache: RwLock::new(HashMap::new()),
        }
    }

    pub async fn refresh(&self) -> Result<(), StorageError> {
        let entries = self
            .store
            .list(ModelNamespace::NAMESPACE, 0, usize::MAX)
            .await?;
        let mut map = HashMap::with_capacity(entries.len());
        for (id, value) in entries {
            let spec: ModelSpec = serde_json::from_value(value)
                .map_err(|e| StorageError::Serialization(e.to_string()))?;
            map.insert(id, spec);
        }
        *self.cache.write().unwrap() = map;
        Ok(())
    }

    pub fn invalidate(&self, id: &str) {
        self.cache.write().unwrap().remove(id);
    }

    pub fn update_cached(&self, spec: ModelSpec) {
        self.cache.write().unwrap().insert(spec.id.clone(), spec);
    }
}

impl ModelRegistry for StoreBackedModelRegistry {
    fn get_model(&self, id: &str) -> Option<ModelSpec> {
        self.cache.read().unwrap().get(id).cloned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use awaken_contract::contract::config_store::ConfigStore;
    use serde_json::Value;
    use tokio::sync::RwLock as TokioRwLock;

    #[derive(Debug, Default)]
    struct MemoryConfigStore {
        data: TokioRwLock<HashMap<String, HashMap<String, Value>>>,
    }

    #[async_trait::async_trait]
    impl ConfigStore for MemoryConfigStore {
        async fn get(&self, namespace: &str, id: &str) -> Result<Option<Value>, StorageError> {
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
            _offset: usize,
            _limit: usize,
        ) -> Result<Vec<(String, Value)>, StorageError> {
            let data = self.data.read().await;
            let Some(ns) = data.get(namespace) else {
                return Ok(Vec::new());
            };
            let mut entries: Vec<_> = ns.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
            entries.sort_by(|a, b| a.0.cmp(&b.0));
            Ok(entries)
        }
        async fn put(&self, namespace: &str, id: &str, value: &Value) -> Result<(), StorageError> {
            self.data
                .write()
                .await
                .entry(namespace.to_string())
                .or_default()
                .insert(id.to_string(), value.clone());
            Ok(())
        }
        async fn delete(&self, namespace: &str, id: &str) -> Result<(), StorageError> {
            if let Some(ns) = self.data.write().await.get_mut(namespace) {
                ns.remove(id);
            }
            Ok(())
        }
    }

    #[tokio::test]
    async fn agent_registry_refresh_and_lookup() {
        let store = Arc::new(MemoryConfigStore::default());
        let spec = AgentSpec {
            id: "coder".into(),
            model: "opus".into(),
            system_prompt: "code".into(),
            ..Default::default()
        };
        store
            .put("agents", "coder", &serde_json::to_value(&spec).unwrap())
            .await
            .unwrap();

        let reg = StoreBackedAgentRegistry::new(store);
        assert!(reg.get_agent("coder").is_none()); // Before refresh

        reg.refresh().await.unwrap();
        let loaded = reg.get_agent("coder").unwrap();
        assert_eq!(loaded.model, "opus");
    }

    #[tokio::test]
    async fn agent_registry_ids_after_refresh() {
        let store = Arc::new(MemoryConfigStore::default());
        for id in ["alpha", "bravo"] {
            let spec = AgentSpec {
                id: id.into(),
                model: "m".into(),
                system_prompt: "sp".into(),
                ..Default::default()
            };
            store
                .put("agents", id, &serde_json::to_value(&spec).unwrap())
                .await
                .unwrap();
        }

        let reg = StoreBackedAgentRegistry::new(store);
        reg.refresh().await.unwrap();

        let mut ids = reg.agent_ids();
        ids.sort();
        assert_eq!(ids, vec!["alpha", "bravo"]);
    }

    #[tokio::test]
    async fn agent_registry_update_cached() {
        let store = Arc::new(MemoryConfigStore::default());
        let reg = StoreBackedAgentRegistry::new(store);

        let spec = AgentSpec {
            id: "a".into(),
            model: "m".into(),
            system_prompt: "v1".into(),
            ..Default::default()
        };
        reg.update_cached(spec);

        let loaded = reg.get_agent("a").unwrap();
        assert_eq!(loaded.system_prompt, "v1");
    }

    #[tokio::test]
    async fn agent_registry_invalidate() {
        let store = Arc::new(MemoryConfigStore::default());
        let reg = StoreBackedAgentRegistry::new(store);

        reg.update_cached(AgentSpec {
            id: "a".into(),
            model: "m".into(),
            system_prompt: "sp".into(),
            ..Default::default()
        });
        assert!(reg.get_agent("a").is_some());

        reg.invalidate("a");
        assert!(reg.get_agent("a").is_none());
    }

    #[tokio::test]
    async fn model_registry_refresh_and_lookup() {
        let store = Arc::new(MemoryConfigStore::default());
        let spec = ModelSpec {
            id: "opus".into(),
            provider: "anthropic".into(),
            model: "claude-opus-4-6".into(),
        };
        store
            .put("models", "opus", &serde_json::to_value(&spec).unwrap())
            .await
            .unwrap();

        let reg = StoreBackedModelRegistry::new(store);
        reg.refresh().await.unwrap();

        let loaded = reg.get_model("opus").unwrap();
        assert_eq!(loaded.provider, "anthropic");
        assert_eq!(loaded.model, "claude-opus-4-6");
    }
}
