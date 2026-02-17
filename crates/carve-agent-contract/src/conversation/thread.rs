//! AgentState (conversation) management.
//!
//! A `AgentState` represents a conversation with messages and state history.

use super::types::Message;
use carve_state::{apply_patches, CarveError, CarveResult, Op, ScopeState, TrackedPatch};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::{Arc, Mutex};

/// Accumulated new messages and patches since the last `take_pending()`.
///
/// This buffer is populated automatically by `with_message`, `with_messages`,
/// `with_patch`, and `with_patches`. Consumers call `take_pending()` to
/// drain the buffer and build a `AgentChangeSet` for storage.
#[derive(Debug, Clone, Default)]
pub struct PendingDelta {
    pub messages: Vec<Arc<Message>>,
    pub patches: Vec<TrackedPatch>,
}

/// Ephemeral runtime-only data used by tools/plugins during a run.
#[derive(Clone)]
pub(crate) struct RuntimeState {
    pub call_id: String,
    pub source: String,
    pub version: u64,
    pub scope_attached: bool,
    pub pending_messages: Arc<Mutex<Vec<Arc<Message>>>>,
    pub ops: Arc<Mutex<Vec<Op>>>,
    pub activity_manager: Option<Arc<dyn crate::context::ActivityManager>>,
}

impl Default for RuntimeState {
    fn default() -> Self {
        Self {
            call_id: String::new(),
            source: String::new(),
            version: 0,
            scope_attached: false,
            pending_messages: Arc::new(Mutex::new(Vec::new())),
            ops: Arc::new(Mutex::new(Vec::new())),
            activity_manager: None,
        }
    }
}

impl std::fmt::Debug for RuntimeState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RuntimeState")
            .field("call_id", &self.call_id)
            .field("source", &self.source)
            .field("version", &self.version)
            .field("scope_attached", &self.scope_attached)
            .field(
                "pending_messages_len",
                &self.pending_messages.lock().unwrap().len(),
            )
            .field("ops_len", &self.ops.lock().unwrap().len())
            .field(
                "activity_manager",
                &self.activity_manager.as_ref().map(|_| "<set>"),
            )
            .finish()
    }
}

impl PendingDelta {
    /// Returns true if there are no pending messages or patches.
    pub fn is_empty(&self) -> bool {
        self.messages.is_empty() && self.patches.is_empty()
    }
}

/// A conversation thread with messages and state history.
///
/// AgentState uses an owned builder pattern: `with_*` methods consume `self`
/// and return a new `AgentState` (e.g., `thread.with_message(msg)`).
///
/// The `scope` field is an exception — it is transient (not serialized)
/// and may be mutated in-place during a run (e.g., setting `run_id`).
/// ScopeState changes are never persisted to storage.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentState {
    /// Unique thread identifier.
    pub id: String,
    /// Owner/resource identifier (e.g., user_id, org_id).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resource_id: Option<String>,
    /// Parent thread identifier (links child → parent for sub-agent lineage).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_thread_id: Option<String>,
    /// Conversation messages (Arc-wrapped for efficient cloning).
    pub messages: Vec<Arc<Message>>,
    /// Initial/snapshot state.
    pub state: Value,
    /// Patches applied since the last snapshot.
    pub patches: Vec<TrackedPatch>,
    /// Metadata.
    #[serde(default)]
    pub metadata: AgentStateMetadata,
    /// Per-run scope context (not persisted).
    #[serde(skip)]
    pub scope: ScopeState,
    /// Pending delta buffer — tracks new items since last `take_pending()`.
    #[serde(skip)]
    pub(crate) pending: PendingDelta,
    /// Runtime-only execution context (not persisted).
    #[serde(skip)]
    pub(crate) runtime: RuntimeState,
}

/// AgentState metadata.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AgentStateMetadata {
    /// Creation timestamp (unix millis).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_at: Option<u64>,
    /// Last update timestamp (unix millis).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<u64>,
    /// Persisted state cursor version.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<u64>,
    /// Timestamp of the latest committed version.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version_timestamp: Option<u64>,
    /// Custom metadata.
    #[serde(flatten)]
    pub extra: serde_json::Map<String, Value>,
}

impl AgentState {
    /// Create a new thread with the given ID.
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            resource_id: None,
            parent_thread_id: None,
            messages: Vec::new(),
            state: Value::Object(serde_json::Map::new()),
            patches: Vec::new(),
            metadata: AgentStateMetadata::default(),
            scope: ScopeState::default(),
            pending: PendingDelta::default(),
            runtime: RuntimeState::default(),
        }
    }

    /// Create a new thread with initial state.
    pub fn with_initial_state(id: impl Into<String>, state: Value) -> Self {
        Self {
            id: id.into(),
            resource_id: None,
            parent_thread_id: None,
            messages: Vec::new(),
            state,
            patches: Vec::new(),
            metadata: AgentStateMetadata::default(),
            scope: ScopeState::default(),
            pending: PendingDelta::default(),
            runtime: RuntimeState::default(),
        }
    }

    /// Set the resource_id (pure function, returns new AgentState).
    #[must_use]
    pub fn with_resource_id(mut self, resource_id: impl Into<String>) -> Self {
        self.resource_id = Some(resource_id.into());
        self
    }

    /// Set the parent_thread_id (pure function, returns new AgentState).
    #[must_use]
    pub fn with_parent_thread_id(mut self, parent_thread_id: impl Into<String>) -> Self {
        self.parent_thread_id = Some(parent_thread_id.into());
        self
    }

    /// Set the scope (pure function, returns new AgentState).
    #[must_use]
    pub fn with_scope(mut self, scope: ScopeState) -> Self {
        self.scope = scope;
        self
    }

    /// Add a message to the thread (pure function, returns new AgentState).
    ///
    /// Messages are Arc-wrapped for efficient cloning during agent loops.
    #[must_use]
    pub fn with_message(mut self, msg: Message) -> Self {
        let arc = Arc::new(msg);
        self.pending.messages.push(arc.clone());
        self.messages.push(arc);
        self
    }

    /// Add multiple messages (pure function, returns new AgentState).
    #[must_use]
    pub fn with_messages(mut self, msgs: impl IntoIterator<Item = Message>) -> Self {
        let arcs: Vec<Arc<Message>> = msgs.into_iter().map(Arc::new).collect();
        self.pending.messages.extend(arcs.iter().cloned());
        self.messages.extend(arcs);
        self
    }

    /// Add a patch to the thread (pure function, returns new AgentState).
    #[must_use]
    pub fn with_patch(mut self, patch: TrackedPatch) -> Self {
        self.pending.patches.push(patch.clone());
        self.patches.push(patch);
        self
    }

    /// Add multiple patches (pure function, returns new AgentState).
    #[must_use]
    pub fn with_patches(mut self, patches: impl IntoIterator<Item = TrackedPatch>) -> Self {
        let patches: Vec<TrackedPatch> = patches.into_iter().collect();
        self.pending.patches.extend(patches.iter().cloned());
        self.patches.extend(patches);
        self
    }

    /// Drain and return the pending delta buffer.
    ///
    /// After this call, the pending buffer is empty. The returned `PendingDelta`
    /// contains all messages and patches added since the last `take_pending()`.
    pub fn take_pending(&mut self) -> PendingDelta {
        std::mem::take(&mut self.pending)
    }

    /// Rebuild the current state by applying all patches to the base state.
    ///
    /// This is a pure function - it doesn't modify the thread.
    pub fn rebuild_state(&self) -> CarveResult<Value> {
        if self.patches.is_empty() {
            return Ok(self.state.clone());
        }

        apply_patches(&self.state, self.patches.iter().map(|p| p.patch()))
    }

    /// Replay state to a specific patch index (0-based).
    ///
    /// - `patch_index = 0`: Returns state after applying the first patch only
    /// - `patch_index = n`: Returns state after applying patches 0..=n
    /// - `patch_index >= patch_count`: Returns error
    ///
    /// This enables time-travel debugging by accessing any historical state point.
    pub fn replay_to(&self, patch_index: usize) -> CarveResult<Value> {
        if patch_index >= self.patches.len() {
            return Err(CarveError::invalid_operation(format!(
                "replay index {patch_index} out of bounds (history len: {})",
                self.patches.len()
            )));
        }

        apply_patches(
            &self.state,
            self.patches[..=patch_index].iter().map(|p| p.patch()),
        )
    }

    /// Create a snapshot, collapsing patches into the base state.
    ///
    /// Returns a new AgentState with the current state as base and empty patches.
    pub fn snapshot(self) -> CarveResult<Self> {
        let current_state = self.rebuild_state()?;
        Ok(Self {
            id: self.id,
            resource_id: self.resource_id,
            parent_thread_id: self.parent_thread_id,
            messages: self.messages,
            state: current_state,
            patches: Vec::new(),
            metadata: self.metadata,
            scope: self.scope,
            pending: self.pending,
            runtime: self.runtime,
        })
    }

    /// Check if a snapshot is needed (e.g., too many patches).
    pub fn needs_snapshot(&self, threshold: usize) -> bool {
        self.patches.len() >= threshold
    }

    /// Get the number of messages.
    pub fn message_count(&self) -> usize {
        self.messages.len()
    }

    /// Get the number of patches.
    pub fn patch_count(&self) -> usize {
        self.patches.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use carve_state::{path, Op, Patch};
    use serde_json::json;

    #[test]
    fn test_pending_delta_tracks_messages() {
        let mut thread = AgentState::new("t-1")
            .with_message(Message::user("Hello"))
            .with_message(Message::assistant("Hi!"));

        assert_eq!(thread.pending.messages.len(), 2);
        assert_eq!(thread.messages.len(), 2);

        let pending = thread.take_pending();
        assert_eq!(pending.messages.len(), 2);
        assert_eq!(pending.messages[0].content, "Hello");
        assert_eq!(pending.messages[1].content, "Hi!");
        assert!(thread.pending.is_empty());
    }

    #[test]
    fn test_pending_delta_tracks_patches() {
        let mut thread = AgentState::new("t-1")
            .with_patch(TrackedPatch::new(
                Patch::new().with_op(Op::set(path!("a"), json!(1))),
            ))
            .with_patches(vec![
                TrackedPatch::new(Patch::new().with_op(Op::set(path!("b"), json!(2)))),
                TrackedPatch::new(Patch::new().with_op(Op::set(path!("c"), json!(3)))),
            ]);

        assert_eq!(thread.pending.patches.len(), 3);
        assert_eq!(thread.patches.len(), 3);

        let pending = thread.take_pending();
        assert_eq!(pending.patches.len(), 3);
        assert!(thread.pending.is_empty());
    }

    #[test]
    fn test_take_pending_resets_buffer() {
        let mut thread = AgentState::new("t-1").with_message(Message::user("first"));
        let p1 = thread.take_pending();
        assert_eq!(p1.messages.len(), 1);

        // Second take should be empty
        let p2 = thread.take_pending();
        assert!(p2.is_empty());

        // Add more, take again
        thread = thread.with_message(Message::user("second"));
        let p3 = thread.take_pending();
        assert_eq!(p3.messages.len(), 1);
        assert_eq!(p3.messages[0].content, "second");
    }

    #[test]
    fn test_pending_delta_not_serialized() {
        let thread = AgentState::new("t-1").with_message(Message::user("Hello"));
        assert_eq!(thread.pending.messages.len(), 1);

        let json_str = serde_json::to_string(&thread).unwrap();
        let restored: AgentState = serde_json::from_str(&json_str).unwrap();
        assert!(
            restored.pending.is_empty(),
            "pending should not survive serialization"
        );
        assert_eq!(restored.messages.len(), 1);
    }

    #[test]
    fn test_pending_clone_is_independent() {
        let thread = AgentState::new("t-1").with_message(Message::user("first"));
        assert_eq!(thread.pending.messages.len(), 1);

        // Clone carries the same pending state
        let mut cloned = thread.clone();
        assert_eq!(cloned.pending.messages.len(), 1);

        // Draining the clone does not affect the original
        let pending = cloned.take_pending();
        assert_eq!(pending.messages.len(), 1);
        assert!(cloned.pending.is_empty());

        // Original still has its pending
        assert_eq!(thread.pending.messages.len(), 1);
    }

    #[test]
    fn test_pending_with_messages_batch() {
        let msgs = vec![
            Message::user("a"),
            Message::assistant("b"),
            Message::user("c"),
        ];
        let mut thread = AgentState::new("t-1").with_messages(msgs);
        assert_eq!(thread.messages.len(), 3);
        assert_eq!(thread.pending.messages.len(), 3);

        let pending = thread.take_pending();
        assert_eq!(pending.messages.len(), 3);
        assert_eq!(pending.messages[0].content, "a");
        assert_eq!(pending.messages[2].content, "c");
    }

    #[test]
    fn test_pending_interleaved_messages_and_patches() {
        let mut thread = AgentState::new("t-1")
            .with_message(Message::user("hello"))
            .with_patch(TrackedPatch::new(
                Patch::new().with_op(Op::set(path!("a"), json!(1))),
            ))
            .with_message(Message::assistant("hi"))
            .with_patches(vec![
                TrackedPatch::new(Patch::new().with_op(Op::set(path!("b"), json!(2)))),
                TrackedPatch::new(Patch::new().with_op(Op::set(path!("c"), json!(3)))),
            ]);

        let pending = thread.take_pending();
        assert_eq!(pending.messages.len(), 2);
        assert_eq!(pending.patches.len(), 3);
        assert!(thread.pending.is_empty());

        // Main arrays still have everything
        assert_eq!(thread.messages.len(), 2);
        assert_eq!(thread.patches.len(), 3);
    }

    #[test]
    fn test_pending_is_empty() {
        let delta = PendingDelta::default();
        assert!(delta.is_empty());

        let delta = PendingDelta {
            messages: vec![Arc::new(Message::user("hi"))],
            patches: vec![],
        };
        assert!(!delta.is_empty());

        let delta = PendingDelta {
            messages: vec![],
            patches: vec![TrackedPatch::new(Patch::new())],
        };
        assert!(!delta.is_empty());
    }

    #[test]
    fn test_thread_new() {
        let thread = AgentState::new("test-1");
        assert_eq!(thread.id, "test-1");
        assert!(thread.resource_id.is_none());
        assert!(thread.messages.is_empty());
        assert!(thread.patches.is_empty());
    }

    #[test]
    fn test_thread_with_resource_id() {
        let thread = AgentState::new("t-1").with_resource_id("user-123");
        assert_eq!(thread.resource_id.as_deref(), Some("user-123"));
    }

    #[test]
    fn test_thread_with_scope() {
        let mut rt = ScopeState::new();
        rt.set("user_id", "u1").unwrap();
        let thread = AgentState::new("t-1").with_scope(rt);
        assert_eq!(thread.scope.value("user_id"), Some(&json!("u1")));
    }

    #[test]
    fn test_scope_is_set_once() {
        let mut thread = AgentState::new("t-1");
        thread.scope.set("run_id", "run-1").unwrap();
        let err = thread.scope.set("run_id", "run-2").unwrap_err();
        assert!(matches!(err, carve_state::ScopeStateError::AlreadySet(key) if key == "run_id"));
        assert_eq!(thread.scope.value("run_id"), Some(&json!("run-1")));
    }

    #[test]
    fn test_thread_with_initial_state() {
        let state = json!({"counter": 0});
        let thread = AgentState::with_initial_state("test-1", state.clone());
        assert_eq!(thread.state, state);
    }

    #[test]
    fn test_thread_with_message() {
        let thread = AgentState::new("test-1")
            .with_message(Message::user("Hello"))
            .with_message(Message::assistant("Hi!"));

        assert_eq!(thread.message_count(), 2);
        assert_eq!(thread.messages[0].content, "Hello");
        assert_eq!(thread.messages[1].content, "Hi!");
    }

    #[test]
    fn test_thread_with_patch() {
        let thread = AgentState::new("test-1");
        let patch = TrackedPatch::new(Patch::new().with_op(Op::set(path!("a"), json!(1))));

        let thread = thread.with_patch(patch);
        assert_eq!(thread.patch_count(), 1);
    }

    #[test]
    fn test_thread_rebuild_state_empty() {
        let state = json!({"counter": 0});
        let thread = AgentState::with_initial_state("test-1", state.clone());

        let rebuilt = thread.rebuild_state().unwrap();
        assert_eq!(rebuilt, state);
    }

    #[test]
    fn test_thread_rebuild_state_with_patches() {
        let state = json!({"counter": 0});
        let thread = AgentState::with_initial_state("test-1", state)
            .with_patch(TrackedPatch::new(
                Patch::new().with_op(Op::set(path!("counter"), json!(1))),
            ))
            .with_patch(TrackedPatch::new(
                Patch::new().with_op(Op::set(path!("name"), json!("test"))),
            ));

        let rebuilt = thread.rebuild_state().unwrap();
        assert_eq!(rebuilt["counter"], 1);
        assert_eq!(rebuilt["name"], "test");
    }

    #[test]
    fn test_thread_snapshot() {
        let state = json!({"counter": 0});
        let thread = AgentState::with_initial_state("test-1", state).with_patch(TrackedPatch::new(
            Patch::new().with_op(Op::set(path!("counter"), json!(5))),
        ));

        assert_eq!(thread.patch_count(), 1);

        let snapshotted = thread.snapshot().unwrap();
        assert_eq!(snapshotted.patch_count(), 0);
        assert_eq!(snapshotted.state["counter"], 5);
    }

    #[test]
    fn test_thread_needs_snapshot() {
        let thread = AgentState::new("test-1");
        assert!(!thread.needs_snapshot(10));

        let thread = (0..10).fold(thread, |s, i| {
            s.with_patch(TrackedPatch::new(
                Patch::new().with_op(Op::set(path!("field").key(i.to_string()), json!(i))),
            ))
        });

        assert!(thread.needs_snapshot(10));
        assert!(!thread.needs_snapshot(20));
    }

    #[test]
    fn test_thread_serialization() {
        let thread = AgentState::new("test-1").with_message(Message::user("Hello"));

        let json_str = serde_json::to_string(&thread).unwrap();
        let restored: AgentState = serde_json::from_str(&json_str).unwrap();

        assert_eq!(restored.id, "test-1");
        assert_eq!(restored.message_count(), 1);
        // ScopeState is not serialized
        assert!(!restored.scope.contains_key("anything"));
    }

    #[test]
    fn test_thread_serialization_skips_scope() {
        let mut rt = ScopeState::new();
        rt.set("token", "secret").unwrap();
        let thread = AgentState::new("test-1").with_scope(rt);

        let json_str = serde_json::to_string(&thread).unwrap();
        assert!(!json_str.contains("secret"));
        assert!(!json_str.contains("scope"));

        // Deserialized thread has default (empty) scope
        let restored: AgentState = serde_json::from_str(&json_str).unwrap();
        assert!(!restored.scope.contains_key("token"));
    }

    #[test]
    fn test_state_persists_but_scope_is_transient_after_serialization() {
        let mut rt = ScopeState::new();
        rt.set("run_id", "run-1").unwrap();

        let thread = AgentState::with_initial_state("test-1", json!({"counter": 0}))
            .with_scope(rt)
            .with_patch(TrackedPatch::new(
                Patch::new().with_op(Op::set(path!("counter"), json!(5))),
            ));

        let json_str = serde_json::to_string(&thread).unwrap();
        let restored: AgentState = serde_json::from_str(&json_str).unwrap();

        let rebuilt = restored.rebuild_state().unwrap();
        assert_eq!(
            rebuilt["counter"], 5,
            "persisted state should survive serialization"
        );
        assert!(
            !restored.scope.contains_key("run_id"),
            "scope data must not be persisted"
        );
    }

    #[test]
    fn test_thread_serialization_includes_resource_id() {
        let thread = AgentState::new("t-1").with_resource_id("org-42");
        let json_str = serde_json::to_string(&thread).unwrap();
        assert!(json_str.contains("org-42"));

        let restored: AgentState = serde_json::from_str(&json_str).unwrap();
        assert_eq!(restored.resource_id.as_deref(), Some("org-42"));
    }

    #[test]
    fn test_thread_replay_to() {
        let state = json!({"counter": 0});
        let thread = AgentState::with_initial_state("test-1", state)
            .with_patch(TrackedPatch::new(
                Patch::new().with_op(Op::set(path!("counter"), json!(10))),
            ))
            .with_patch(TrackedPatch::new(
                Patch::new().with_op(Op::set(path!("counter"), json!(20))),
            ))
            .with_patch(TrackedPatch::new(
                Patch::new().with_op(Op::set(path!("counter"), json!(30))),
            ));

        let state_at_0 = thread.replay_to(0).unwrap();
        assert_eq!(state_at_0["counter"], 10);

        let state_at_1 = thread.replay_to(1).unwrap();
        assert_eq!(state_at_1["counter"], 20);

        let state_at_2 = thread.replay_to(2).unwrap();
        assert_eq!(state_at_2["counter"], 30);

        let err = thread.replay_to(100).unwrap_err();
        assert!(err
            .to_string()
            .contains("replay index 100 out of bounds (history len: 3)"));
    }

    #[test]
    fn test_thread_replay_to_empty() {
        let state = json!({"counter": 0});
        let thread = AgentState::with_initial_state("test-1", state.clone());

        let err = thread.replay_to(0).unwrap_err();
        assert!(err
            .to_string()
            .contains("replay index 0 out of bounds (history len: 0)"));
    }
}
