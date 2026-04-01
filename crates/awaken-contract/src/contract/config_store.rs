//! Unified configuration store for named JSON documents.
//!
//! All persistable configuration entities (agents, models, providers, MCP servers)
//! share the same storage abstraction: `(namespace, id) → JSON value`.
//!
//! See ADR-0021 for design rationale.

use async_trait::async_trait;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

use super::storage::StorageError;

/// Async CRUD store for namespaced JSON configuration documents.
///
/// Each entry is identified by `(namespace, id)` and stored as a JSON value.
/// Implementations may use files, databases, or in-memory maps.
#[async_trait]
pub trait ConfigStore: Send + Sync {
    /// Get a single entry by namespace and ID.
    async fn get(&self, namespace: &str, id: &str) -> Result<Option<Value>, StorageError>;

    /// List entries in a namespace with pagination.
    ///
    /// Returns `(id, value)` pairs ordered by ID.
    async fn list(
        &self,
        namespace: &str,
        offset: usize,
        limit: usize,
    ) -> Result<Vec<(String, Value)>, StorageError>;

    /// Create or overwrite an entry.
    async fn put(&self, namespace: &str, id: &str, value: &Value) -> Result<(), StorageError>;

    /// Delete an entry. Returns `Ok(())` even if the entry did not exist.
    async fn delete(&self, namespace: &str, id: &str) -> Result<(), StorageError>;

    /// Check whether an entry exists.
    async fn exists(&self, namespace: &str, id: &str) -> Result<bool, StorageError> {
        Ok(self.get(namespace, id).await?.is_some())
    }
}

/// Typed namespace descriptor for a configuration entity.
///
/// Maps a namespace string to a concrete Rust type with JSON Schema support.
/// Used by [`ConfigRegistry`] to provide typed access over a raw [`ConfigStore`].
pub trait ConfigNamespace: 'static + Send + Sync {
    /// Storage namespace (e.g. `"agents"`, `"models"`).
    const NAMESPACE: &'static str;

    /// The typed value for this namespace.
    type Value: Serialize + DeserializeOwned + Send + Sync + 'static;

    /// Extract the unique ID from a value (e.g. `AgentSpec.id`).
    fn id(value: &Self::Value) -> &str;
}

/// Typed wrapper over [`ConfigStore`] for a specific [`ConfigNamespace`].
///
/// Provides serialization/deserialization and optional JSON Schema validation.
pub struct ConfigRegistry<N: ConfigNamespace> {
    store: std::sync::Arc<dyn ConfigStore>,
    _ns: std::marker::PhantomData<N>,
}

impl<N: ConfigNamespace> ConfigRegistry<N> {
    /// Create a new typed registry backed by the given store.
    pub fn new(store: std::sync::Arc<dyn ConfigStore>) -> Self {
        Self {
            store,
            _ns: std::marker::PhantomData,
        }
    }

    /// Get a typed entry by ID.
    pub async fn get(&self, id: &str) -> Result<Option<N::Value>, StorageError> {
        let Some(value) = self.store.get(N::NAMESPACE, id).await? else {
            return Ok(None);
        };
        let typed = serde_json::from_value(value)
            .map_err(|e| StorageError::Serialization(e.to_string()))?;
        Ok(Some(typed))
    }

    /// List typed entries with pagination.
    pub async fn list(&self, offset: usize, limit: usize) -> Result<Vec<N::Value>, StorageError> {
        let entries = self.store.list(N::NAMESPACE, offset, limit).await?;
        entries
            .into_iter()
            .map(|(_, v)| {
                serde_json::from_value(v).map_err(|e| StorageError::Serialization(e.to_string()))
            })
            .collect()
    }

    /// Create or update a typed entry.
    pub async fn put(&self, value: &N::Value) -> Result<(), StorageError> {
        let id = N::id(value);
        let json =
            serde_json::to_value(value).map_err(|e| StorageError::Serialization(e.to_string()))?;
        self.store.put(N::NAMESPACE, id, &json).await
    }

    /// Delete an entry by ID.
    pub async fn delete(&self, id: &str) -> Result<(), StorageError> {
        self.store.delete(N::NAMESPACE, id).await
    }

    /// Check existence by ID.
    pub async fn exists(&self, id: &str) -> Result<bool, StorageError> {
        self.store.exists(N::NAMESPACE, id).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::{Deserialize, Serialize};
    use std::collections::HashMap;
    use std::sync::Arc;
    use tokio::sync::RwLock;

    // -- In-memory ConfigStore for testing --

    #[derive(Debug, Default)]
    struct MemoryConfigStore {
        data: RwLock<HashMap<String, HashMap<String, Value>>>,
    }

    #[async_trait]
    impl ConfigStore for MemoryConfigStore {
        async fn get(&self, namespace: &str, id: &str) -> Result<Option<Value>, StorageError> {
            let data = self.data.read().await;
            Ok(data.get(namespace).and_then(|ns| ns.get(id)).cloned())
        }

        async fn list(
            &self,
            namespace: &str,
            offset: usize,
            limit: usize,
        ) -> Result<Vec<(String, Value)>, StorageError> {
            let data = self.data.read().await;
            let Some(ns) = data.get(namespace) else {
                return Ok(Vec::new());
            };
            let mut entries: Vec<_> = ns.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
            entries.sort_by(|a, b| a.0.cmp(&b.0));
            Ok(entries.into_iter().skip(offset).take(limit).collect())
        }

        async fn put(&self, namespace: &str, id: &str, value: &Value) -> Result<(), StorageError> {
            let mut data = self.data.write().await;
            data.entry(namespace.to_string())
                .or_default()
                .insert(id.to_string(), value.clone());
            Ok(())
        }

        async fn delete(&self, namespace: &str, id: &str) -> Result<(), StorageError> {
            let mut data = self.data.write().await;
            if let Some(ns) = data.get_mut(namespace) {
                ns.remove(id);
            }
            Ok(())
        }
    }

    // -- Test namespace --

    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    struct TestEntity {
        id: String,
        name: String,
        value: u32,
    }

    struct TestNamespace;
    impl ConfigNamespace for TestNamespace {
        const NAMESPACE: &'static str = "test-entities";
        type Value = TestEntity;
        fn id(value: &TestEntity) -> &str {
            &value.id
        }
    }

    #[tokio::test]
    async fn put_and_get() {
        let store = Arc::new(MemoryConfigStore::default());
        let reg = ConfigRegistry::<TestNamespace>::new(store);

        let entity = TestEntity {
            id: "e1".into(),
            name: "first".into(),
            value: 42,
        };
        reg.put(&entity).await.unwrap();

        let got = reg.get("e1").await.unwrap().unwrap();
        assert_eq!(got, entity);
    }

    #[tokio::test]
    async fn get_missing_returns_none() {
        let store = Arc::new(MemoryConfigStore::default());
        let reg = ConfigRegistry::<TestNamespace>::new(store);
        assert!(reg.get("missing").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn put_overwrites() {
        let store = Arc::new(MemoryConfigStore::default());
        let reg = ConfigRegistry::<TestNamespace>::new(store);

        reg.put(&TestEntity {
            id: "e1".into(),
            name: "v1".into(),
            value: 1,
        })
        .await
        .unwrap();
        reg.put(&TestEntity {
            id: "e1".into(),
            name: "v2".into(),
            value: 2,
        })
        .await
        .unwrap();

        let got = reg.get("e1").await.unwrap().unwrap();
        assert_eq!(got.name, "v2");
        assert_eq!(got.value, 2);
    }

    #[tokio::test]
    async fn delete_and_verify() {
        let store = Arc::new(MemoryConfigStore::default());
        let reg = ConfigRegistry::<TestNamespace>::new(store);

        reg.put(&TestEntity {
            id: "e1".into(),
            name: "x".into(),
            value: 1,
        })
        .await
        .unwrap();
        reg.delete("e1").await.unwrap();

        assert!(reg.get("e1").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn delete_nonexistent_is_ok() {
        let store = Arc::new(MemoryConfigStore::default());
        let reg = ConfigRegistry::<TestNamespace>::new(store);
        // Should not error
        reg.delete("nonexistent").await.unwrap();
    }

    #[tokio::test]
    async fn list_with_pagination() {
        let store = Arc::new(MemoryConfigStore::default());
        let reg = ConfigRegistry::<TestNamespace>::new(store);

        for i in 0..5 {
            reg.put(&TestEntity {
                id: format!("e{i}"),
                name: format!("name{i}"),
                value: i,
            })
            .await
            .unwrap();
        }

        let page = reg.list(1, 2).await.unwrap();
        assert_eq!(page.len(), 2);
        assert_eq!(page[0].id, "e1");
        assert_eq!(page[1].id, "e2");
    }

    #[tokio::test]
    async fn list_empty_namespace() {
        let store = Arc::new(MemoryConfigStore::default());
        let reg = ConfigRegistry::<TestNamespace>::new(store);
        let page = reg.list(0, 100).await.unwrap();
        assert!(page.is_empty());
    }

    #[tokio::test]
    async fn exists_check() {
        let store = Arc::new(MemoryConfigStore::default());
        let reg = ConfigRegistry::<TestNamespace>::new(store);

        assert!(!reg.exists("e1").await.unwrap());
        reg.put(&TestEntity {
            id: "e1".into(),
            name: "x".into(),
            value: 1,
        })
        .await
        .unwrap();
        assert!(reg.exists("e1").await.unwrap());
    }

    #[tokio::test]
    async fn namespaces_are_isolated() {
        let store = Arc::new(MemoryConfigStore::default());

        // Use raw store to put in different namespaces
        store
            .put("ns-a", "id1", &serde_json::json!({"a": 1}))
            .await
            .unwrap();
        store
            .put("ns-b", "id1", &serde_json::json!({"b": 2}))
            .await
            .unwrap();

        let a = store.get("ns-a", "id1").await.unwrap().unwrap();
        let b = store.get("ns-b", "id1").await.unwrap().unwrap();
        assert_eq!(a["a"], 1);
        assert_eq!(b["b"], 2);

        // Delete from ns-a doesn't affect ns-b
        store.delete("ns-a", "id1").await.unwrap();
        assert!(store.get("ns-a", "id1").await.unwrap().is_none());
        assert!(store.get("ns-b", "id1").await.unwrap().is_some());
    }

    // -- AgentSpec roundtrip test --

    struct AgentNamespace;
    impl ConfigNamespace for AgentNamespace {
        const NAMESPACE: &'static str = "agents";
        type Value = crate::AgentSpec;
        fn id(value: &crate::AgentSpec) -> &str {
            &value.id
        }
    }

    #[tokio::test]
    async fn agent_spec_roundtrip_through_config_store() {
        use crate::AgentSpec;

        let store = Arc::new(MemoryConfigStore::default());
        let reg = ConfigRegistry::<AgentNamespace>::new(store);

        let spec = AgentSpec {
            id: "coder".into(),
            model: "claude-opus".into(),
            system_prompt: "You code.".into(),
            plugin_ids: vec!["permission".into()],
            sections: {
                let mut m = std::collections::HashMap::new();
                m.insert(
                    "permission".into(),
                    serde_json::json!({"default_behavior": "ask"}),
                );
                m
            },
            ..Default::default()
        };

        reg.put(&spec).await.unwrap();
        let restored = reg.get("coder").await.unwrap().unwrap();

        assert_eq!(restored.id, "coder");
        assert_eq!(restored.model, "claude-opus");
        assert_eq!(restored.plugin_ids, vec!["permission"]);
        assert_eq!(restored.sections["permission"]["default_behavior"], "ask");
    }
}
