//! Thread (conversation) management for Agent sessions.
//!
//! A `Thread` represents a conversation with messages and state history.
//! The `Session` type alias is provided for backward compatibility.

use crate::types::Message;
use carve_state::{apply_patches, CarveResult, Runtime, TrackedPatch};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// A conversation thread with messages and state history.
///
/// Thread is an immutable data structure. All modification methods
/// return a new Thread instance (functional style).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Thread {
    /// Unique thread identifier.
    pub id: String,
    /// Owner/resource identifier (e.g., user_id, org_id).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resource_id: Option<String>,
    /// Conversation messages.
    pub messages: Vec<Message>,
    /// Initial/snapshot state.
    pub state: Value,
    /// Patches applied since the last snapshot.
    pub patches: Vec<TrackedPatch>,
    /// Metadata.
    #[serde(default)]
    pub metadata: ThreadMetadata,
    /// Per-run runtime context (not persisted).
    #[serde(skip)]
    pub runtime: Runtime,
}

/// Thread metadata.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ThreadMetadata {
    /// Creation timestamp (unix millis).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_at: Option<u64>,
    /// Last update timestamp (unix millis).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<u64>,
    /// Custom metadata.
    #[serde(flatten)]
    pub extra: serde_json::Map<String, Value>,
}

/// Backward-compatible alias.
pub type Session = Thread;
/// Backward-compatible alias.
pub type SessionMetadata = ThreadMetadata;

impl Thread {
    /// Create a new thread with the given ID.
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            resource_id: None,
            messages: Vec::new(),
            state: Value::Object(serde_json::Map::new()),
            patches: Vec::new(),
            metadata: ThreadMetadata::default(),
            runtime: Runtime::default(),
        }
    }

    /// Create a new thread with initial state.
    pub fn with_initial_state(id: impl Into<String>, state: Value) -> Self {
        Self {
            id: id.into(),
            resource_id: None,
            messages: Vec::new(),
            state,
            patches: Vec::new(),
            metadata: ThreadMetadata::default(),
            runtime: Runtime::default(),
        }
    }

    /// Set the resource_id (pure function, returns new Thread).
    #[must_use]
    pub fn with_resource_id(mut self, resource_id: impl Into<String>) -> Self {
        self.resource_id = Some(resource_id.into());
        self
    }

    /// Set the runtime (pure function, returns new Thread).
    #[must_use]
    pub fn with_runtime(mut self, runtime: Runtime) -> Self {
        self.runtime = runtime;
        self
    }

    /// Add a message to the thread (pure function, returns new Thread).
    #[must_use]
    pub fn with_message(mut self, msg: Message) -> Self {
        self.messages.push(msg);
        self
    }

    /// Add multiple messages (pure function, returns new Thread).
    #[must_use]
    pub fn with_messages(mut self, msgs: impl IntoIterator<Item = Message>) -> Self {
        self.messages.extend(msgs);
        self
    }

    /// Add a patch to the thread (pure function, returns new Thread).
    #[must_use]
    pub fn with_patch(mut self, patch: TrackedPatch) -> Self {
        self.patches.push(patch);
        self
    }

    /// Add multiple patches (pure function, returns new Thread).
    #[must_use]
    pub fn with_patches(mut self, patches: impl IntoIterator<Item = TrackedPatch>) -> Self {
        self.patches.extend(patches);
        self
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
    ///
    /// This enables time-travel debugging by accessing any historical state point.
    pub fn replay_to(&self, patch_index: usize) -> CarveResult<Value> {
        if self.patches.is_empty() || patch_index >= self.patches.len() {
            return self.rebuild_state();
        }

        apply_patches(
            &self.state,
            self.patches[..=patch_index].iter().map(|p| p.patch()),
        )
    }

    /// Create a snapshot, collapsing patches into the base state.
    ///
    /// Returns a new Thread with the current state as base and empty patches.
    pub fn snapshot(self) -> CarveResult<Self> {
        let current_state = self.rebuild_state()?;
        Ok(Self {
            id: self.id,
            resource_id: self.resource_id,
            messages: self.messages,
            state: current_state,
            patches: Vec::new(),
            metadata: self.metadata,
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
    fn test_thread_new() {
        let thread = Thread::new("test-1");
        assert_eq!(thread.id, "test-1");
        assert!(thread.resource_id.is_none());
        assert!(thread.messages.is_empty());
        assert!(thread.patches.is_empty());
    }

    #[test]
    fn test_thread_with_resource_id() {
        let thread = Thread::new("t-1").with_resource_id("user-123");
        assert_eq!(thread.resource_id.as_deref(), Some("user-123"));
    }

    #[test]
    fn test_thread_with_runtime() {
        let mut rt = Runtime::new();
        rt.set("user_id", "u1").unwrap();
        let thread = Thread::new("t-1").with_runtime(rt);
        assert_eq!(thread.runtime.value("user_id"), Some(&json!("u1")));
    }

    #[test]
    fn test_session_alias() {
        // Session is a type alias for Thread
        let session: Session = Session::new("test-1");
        assert_eq!(session.id, "test-1");
    }

    #[test]
    fn test_thread_with_initial_state() {
        let state = json!({"counter": 0});
        let thread = Thread::with_initial_state("test-1", state.clone());
        assert_eq!(thread.state, state);
    }

    #[test]
    fn test_thread_with_message() {
        let thread = Thread::new("test-1")
            .with_message(Message::user("Hello"))
            .with_message(Message::assistant("Hi!"));

        assert_eq!(thread.message_count(), 2);
        assert_eq!(thread.messages[0].content, "Hello");
        assert_eq!(thread.messages[1].content, "Hi!");
    }

    #[test]
    fn test_thread_with_patch() {
        let thread = Thread::new("test-1");
        let patch = TrackedPatch::new(Patch::new().with_op(Op::set(path!("a"), json!(1))));

        let thread = thread.with_patch(patch);
        assert_eq!(thread.patch_count(), 1);
    }

    #[test]
    fn test_thread_rebuild_state_empty() {
        let state = json!({"counter": 0});
        let thread = Thread::with_initial_state("test-1", state.clone());

        let rebuilt = thread.rebuild_state().unwrap();
        assert_eq!(rebuilt, state);
    }

    #[test]
    fn test_thread_rebuild_state_with_patches() {
        let state = json!({"counter": 0});
        let thread = Thread::with_initial_state("test-1", state)
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
        let thread = Thread::with_initial_state("test-1", state).with_patch(TrackedPatch::new(
            Patch::new().with_op(Op::set(path!("counter"), json!(5))),
        ));

        assert_eq!(thread.patch_count(), 1);

        let snapshotted = thread.snapshot().unwrap();
        assert_eq!(snapshotted.patch_count(), 0);
        assert_eq!(snapshotted.state["counter"], 5);
    }

    #[test]
    fn test_thread_needs_snapshot() {
        let thread = Thread::new("test-1");
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
        let thread = Thread::new("test-1").with_message(Message::user("Hello"));

        let json_str = serde_json::to_string(&thread).unwrap();
        let restored: Thread = serde_json::from_str(&json_str).unwrap();

        assert_eq!(restored.id, "test-1");
        assert_eq!(restored.message_count(), 1);
        // Runtime is not serialized
        assert!(!restored.runtime.contains_key("anything"));
    }

    #[test]
    fn test_thread_serialization_skips_runtime() {
        let mut rt = Runtime::new();
        rt.set("token", "secret").unwrap();
        let thread = Thread::new("test-1").with_runtime(rt);

        let json_str = serde_json::to_string(&thread).unwrap();
        assert!(!json_str.contains("secret"));
        assert!(!json_str.contains("runtime"));

        // Deserialized thread has default (empty) runtime
        let restored: Thread = serde_json::from_str(&json_str).unwrap();
        assert!(!restored.runtime.contains_key("token"));
    }

    #[test]
    fn test_thread_serialization_includes_resource_id() {
        let thread = Thread::new("t-1").with_resource_id("org-42");
        let json_str = serde_json::to_string(&thread).unwrap();
        assert!(json_str.contains("org-42"));

        let restored: Thread = serde_json::from_str(&json_str).unwrap();
        assert_eq!(restored.resource_id.as_deref(), Some("org-42"));
    }

    #[test]
    fn test_thread_replay_to() {
        let state = json!({"counter": 0});
        let thread = Thread::with_initial_state("test-1", state)
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

        let state_beyond = thread.replay_to(100).unwrap();
        assert_eq!(state_beyond["counter"], 30);
    }

    #[test]
    fn test_thread_replay_to_empty() {
        let state = json!({"counter": 0});
        let thread = Thread::with_initial_state("test-1", state.clone());

        let replayed = thread.replay_to(0).unwrap();
        assert_eq!(replayed, state);
    }
}
