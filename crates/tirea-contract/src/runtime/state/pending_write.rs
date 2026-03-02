use super::spec::{AnyStateAction, StateScope};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Mutex;
use tirea_state::StateSpec;

/// Serialized state action, sufficient to reconstruct an [`AnyStateAction::Typed`].
///
/// Captured at the point where a tool completes execution, before the batch
/// commit. On crash recovery, these entries are deserialized back into
/// `AnyStateAction` via [`ActionDeserializerRegistry`] and re-reduced against
/// the base state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SerializedAction {
    /// `std::any::type_name::<S>()` — used as the registry lookup key.
    pub state_type_name: String,
    /// `S::PATH` — the canonical JSON path for this state type.
    pub base_path: String,
    /// Whether this action targets run-level or tool-call-level state.
    pub scope: StateScope,
    /// When set, overrides the scope context call_id for path resolution.
    pub call_id_override: Option<String>,
    /// The serialized `S::Action` value.
    pub payload: Value,
}

/// A single tool call's pending state writes.
///
/// Persisted immediately after a tool completes execution, before the batch
/// commit. On recovery, all entries for a run are read and replayed.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingWriteEntry {
    pub run_id: String,
    pub thread_id: String,
    pub call_id: String,
    pub tool_name: String,
    pub actions: Vec<SerializedAction>,
    pub created_at: u64,
}

/// Errors from pending-write operations.
#[derive(Debug, thiserror::Error)]
pub enum PendingWriteError {
    #[error("unknown state type: {0}")]
    UnknownStateType(String),
    #[error("action deserialization failed for {state_type}: {source}")]
    DeserializationFailed {
        state_type: String,
        source: serde_json::Error,
    },
    #[error("store I/O error: {0}")]
    Store(String),
}

// ---------------------------------------------------------------------------
// AnyStateAction → SerializedAction
// ---------------------------------------------------------------------------

impl AnyStateAction {
    /// Convert this action into a serialized form for pending-write persistence.
    ///
    /// Returns `None` for raw `Patch` actions (they bypass the typed reducer
    /// pipeline and are not recoverable via action replay).
    pub fn to_serialized_action(&self) -> Option<SerializedAction> {
        match self {
            Self::Typed {
                state_type_name,
                scope,
                base_path,
                call_id_override,
                serialized_payload,
                ..
            } => Some(SerializedAction {
                state_type_name: (*state_type_name).to_owned(),
                base_path: (*base_path).to_owned(),
                scope: *scope,
                call_id_override: call_id_override.clone(),
                payload: serialized_payload.clone(),
            }),
            Self::Patch(_) => None,
        }
    }
}

// ---------------------------------------------------------------------------
// ActionDeserializerRegistry
// ---------------------------------------------------------------------------

type ActionFactory =
    Box<dyn Fn(&SerializedAction) -> Result<AnyStateAction, PendingWriteError> + Send + Sync>;

/// Registry that maps `state_type_name` → factory closure for reconstructing
/// `AnyStateAction` from a [`SerializedAction`].
///
/// Built once at agent construction (alongside `StateScopeRegistry` and
/// `LatticeRegistry`) by calling `register::<S>()` for every `StateSpec` type.
pub struct ActionDeserializerRegistry {
    factories: HashMap<String, ActionFactory>,
}

impl ActionDeserializerRegistry {
    pub fn new() -> Self {
        Self {
            factories: HashMap::new(),
        }
    }

    /// Register a `StateSpec` type so its actions can be deserialized from
    /// pending writes.
    pub fn register<S: StateSpec>(&mut self) {
        let type_name = std::any::type_name::<S>().to_owned();
        self.factories.insert(
            type_name,
            Box::new(|entry: &SerializedAction| {
                let action: S::Action = serde_json::from_value(entry.payload.clone())
                    .map_err(|e| PendingWriteError::DeserializationFailed {
                        state_type: entry.state_type_name.clone(),
                        source: e,
                    })?;
                match entry.scope {
                    StateScope::Run => Ok(AnyStateAction::new::<S>(action)),
                    StateScope::ToolCall => {
                        let call_id = entry.call_id_override.as_deref().unwrap_or("");
                        Ok(AnyStateAction::new_for_call::<S>(
                            action,
                            call_id.to_owned(),
                        ))
                    }
                }
            }),
        );
    }

    /// Deserialize a [`SerializedAction`] back into an [`AnyStateAction`].
    pub fn deserialize(
        &self,
        entry: &SerializedAction,
    ) -> Result<AnyStateAction, PendingWriteError> {
        let factory = self
            .factories
            .get(&entry.state_type_name)
            .ok_or_else(|| PendingWriteError::UnknownStateType(entry.state_type_name.clone()))?;
        factory(entry)
    }
}

impl Default for ActionDeserializerRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for ActionDeserializerRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ActionDeserializerRegistry")
            .field("registered_types", &self.factories.keys().collect::<Vec<_>>())
            .finish()
    }
}

// ---------------------------------------------------------------------------
// PendingWriteStore
// ---------------------------------------------------------------------------

/// Durable store for per-tool pending writes.
///
/// Separated from `StateCommitter` — no version precondition, no conflict
/// with the batch commit path. Implementations can be backed by the
/// filesystem, a database, or in-memory (for tests).
#[async_trait]
pub trait PendingWriteStore: Send + Sync {
    /// Persist a single pending-write entry (called immediately after a tool completes).
    async fn write(&self, entry: PendingWriteEntry) -> Result<(), PendingWriteError>;

    /// Read all pending writes for a given run (called during recovery).
    async fn read(
        &self,
        thread_id: &str,
        run_id: &str,
    ) -> Result<Vec<PendingWriteEntry>, PendingWriteError>;

    /// Acknowledge (delete) all pending writes for a run after successful batch commit.
    async fn acknowledge(
        &self,
        thread_id: &str,
        run_id: &str,
    ) -> Result<(), PendingWriteError>;
}

// ---------------------------------------------------------------------------
// InMemoryPendingWriteStore (test / development use)
// ---------------------------------------------------------------------------

/// In-memory implementation of [`PendingWriteStore`] for testing.
#[derive(Debug, Default)]
pub struct InMemoryPendingWriteStore {
    /// Key: `(thread_id, run_id)` → entries
    entries: Mutex<HashMap<(String, String), Vec<PendingWriteEntry>>>,
}

impl InMemoryPendingWriteStore {
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl PendingWriteStore for InMemoryPendingWriteStore {
    async fn write(&self, entry: PendingWriteEntry) -> Result<(), PendingWriteError> {
        let key = (entry.thread_id.clone(), entry.run_id.clone());
        self.entries
            .lock()
            .map_err(|e| PendingWriteError::Store(e.to_string()))?
            .entry(key)
            .or_default()
            .push(entry);
        Ok(())
    }

    async fn read(
        &self,
        thread_id: &str,
        run_id: &str,
    ) -> Result<Vec<PendingWriteEntry>, PendingWriteError> {
        let key = (thread_id.to_owned(), run_id.to_owned());
        Ok(self
            .entries
            .lock()
            .map_err(|e| PendingWriteError::Store(e.to_string()))?
            .get(&key)
            .cloned()
            .unwrap_or_default())
    }

    async fn acknowledge(
        &self,
        thread_id: &str,
        run_id: &str,
    ) -> Result<(), PendingWriteError> {
        let key = (thread_id.to_owned(), run_id.to_owned());
        self.entries
            .lock()
            .map_err(|e| PendingWriteError::Store(e.to_string()))?
            .remove(&key);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::{Deserialize, Serialize};
    use serde_json::json;
    use tirea_state::{
        apply_patch, DocCell, PatchSink, Path, State, TireaResult,
    };

    use super::super::scope_context::ScopeContext;
    use super::super::spec::reduce_state_actions;

    // -- Test state type --

    #[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
    struct TestCounter {
        value: i64,
    }

    struct TestCounterRef;

    impl State for TestCounter {
        type Ref<'a> = TestCounterRef;
        const PATH: &'static str = "test_counter";

        fn state_ref<'a>(_: &'a DocCell, _: Path, _: PatchSink<'a>) -> Self::Ref<'a> {
            TestCounterRef
        }

        fn from_value(value: &Value) -> TireaResult<Self> {
            if value.is_null() {
                return Ok(Self::default());
            }
            serde_json::from_value(value.clone()).map_err(tirea_state::TireaError::Serialization)
        }

        fn to_value(&self) -> TireaResult<Value> {
            serde_json::to_value(self).map_err(tirea_state::TireaError::Serialization)
        }
    }

    #[derive(Debug, Serialize, Deserialize)]
    enum TestCounterAction {
        Increment(i64),
        Reset,
    }

    impl StateSpec for TestCounter {
        type Action = TestCounterAction;

        fn reduce(&mut self, action: TestCounterAction) {
            match action {
                TestCounterAction::Increment(n) => self.value += n,
                TestCounterAction::Reset => self.value = 0,
            }
        }
    }

    #[test]
    fn to_serialized_action_roundtrip() {
        let original = AnyStateAction::new::<TestCounter>(TestCounterAction::Increment(42));
        let serialized = original.to_serialized_action().expect("Typed → Some");

        assert!(serialized.state_type_name.contains("TestCounter"));
        assert_eq!(serialized.base_path, "test_counter");
        assert_eq!(serialized.scope, StateScope::Run);
        assert!(serialized.call_id_override.is_none());
        assert_eq!(serialized.payload, json!({"Increment": 42}));
    }

    #[test]
    fn to_serialized_action_returns_none_for_patch() {
        let raw = AnyStateAction::Patch(tirea_state::TrackedPatch::new(
            tirea_state::Patch::default(),
        ));
        assert!(raw.to_serialized_action().is_none());
    }

    #[test]
    fn registry_deserialize_and_reduce_roundtrip() {
        let mut registry = ActionDeserializerRegistry::new();
        registry.register::<TestCounter>();

        // Create original action, serialize, then deserialize through registry
        let original = AnyStateAction::new::<TestCounter>(TestCounterAction::Increment(7));
        let serialized = original.to_serialized_action().unwrap();

        let reconstructed = registry.deserialize(&serialized).unwrap();

        // Reduce both against the same base state
        let base = json!({});
        let original_patches = reduce_state_actions(
            vec![original],
            &base,
            "test",
            &ScopeContext::run(),
        )
        .unwrap();
        let reconstructed_patches = reduce_state_actions(
            vec![reconstructed],
            &base,
            "test",
            &ScopeContext::run(),
        )
        .unwrap();

        // Both should produce identical results
        let result_a = apply_patch(&base, original_patches[0].patch()).unwrap();
        let result_b = apply_patch(&base, reconstructed_patches[0].patch()).unwrap();
        assert_eq!(result_a, result_b);
        assert_eq!(result_a["test_counter"]["value"], 7);
    }

    #[test]
    fn registry_unknown_type_returns_error() {
        let registry = ActionDeserializerRegistry::new();
        let entry = SerializedAction {
            state_type_name: "unknown::Type".into(),
            base_path: "x".into(),
            scope: StateScope::Run,
            call_id_override: None,
            payload: json!(null),
        };
        let err = registry.deserialize(&entry).unwrap_err();
        assert!(matches!(err, PendingWriteError::UnknownStateType(_)));
    }

    #[test]
    fn registry_bad_payload_returns_deserialization_error() {
        let mut registry = ActionDeserializerRegistry::new();
        registry.register::<TestCounter>();

        let entry = SerializedAction {
            state_type_name: std::any::type_name::<TestCounter>().into(),
            base_path: "test_counter".into(),
            scope: StateScope::Run,
            call_id_override: None,
            payload: json!({"BadVariant": 99}),
        };
        let err = registry.deserialize(&entry).unwrap_err();
        assert!(matches!(err, PendingWriteError::DeserializationFailed { .. }));
    }

    #[test]
    fn tool_call_scoped_roundtrip() {
        let mut registry = ActionDeserializerRegistry::new();
        registry.register::<TestCounter>();

        let original = AnyStateAction::new_for_call::<TestCounter>(
            TestCounterAction::Increment(3),
            "call_99",
        );
        let serialized = original.to_serialized_action().unwrap();
        assert_eq!(serialized.scope, StateScope::ToolCall);
        assert_eq!(serialized.call_id_override, Some("call_99".into()));

        let reconstructed = registry.deserialize(&serialized).unwrap();

        let base = json!({});
        let patches = reduce_state_actions(
            vec![reconstructed],
            &base,
            "test",
            &ScopeContext::run(),
        )
        .unwrap();
        let result = apply_patch(&base, patches[0].patch()).unwrap();
        assert_eq!(
            result["__tool_call_scope"]["call_99"]["test_counter"]["value"],
            3
        );
    }

    #[tokio::test]
    async fn in_memory_store_write_read_acknowledge() {
        let store = InMemoryPendingWriteStore::new();
        let entry = PendingWriteEntry {
            run_id: "run_1".into(),
            thread_id: "thread_1".into(),
            call_id: "call_1".into(),
            tool_name: "my_tool".into(),
            actions: vec![SerializedAction {
                state_type_name: "TestCounter".into(),
                base_path: "test_counter".into(),
                scope: StateScope::Run,
                call_id_override: None,
                payload: json!({"Increment": 1}),
            }],
            created_at: 1000,
        };

        store.write(entry).await.unwrap();

        let entries = store.read("thread_1", "run_1").await.unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].call_id, "call_1");
        assert_eq!(entries[0].actions.len(), 1);

        // Unrelated run returns empty
        let empty = store.read("thread_1", "run_other").await.unwrap();
        assert!(empty.is_empty());

        // Acknowledge removes entries
        store.acknowledge("thread_1", "run_1").await.unwrap();
        let after = store.read("thread_1", "run_1").await.unwrap();
        assert!(after.is_empty());
    }

    #[tokio::test]
    async fn in_memory_store_multiple_entries() {
        let store = InMemoryPendingWriteStore::new();

        for i in 0..3 {
            store
                .write(PendingWriteEntry {
                    run_id: "run_1".into(),
                    thread_id: "t1".into(),
                    call_id: format!("call_{i}"),
                    tool_name: "tool".into(),
                    actions: vec![],
                    created_at: i as u64,
                })
                .await
                .unwrap();
        }

        let entries = store.read("t1", "run_1").await.unwrap();
        assert_eq!(entries.len(), 3);
    }
}
