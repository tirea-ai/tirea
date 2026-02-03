//! Session management for Agent conversations.

use crate::types::Message;
use carve_state::{apply_patches, CarveResult, TrackedPatch};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// A conversation session with messages and state history.
///
/// Session is an immutable data structure. All modification methods
/// return a new Session instance (functional style).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    /// Unique session identifier.
    pub id: String,
    /// Conversation messages.
    pub messages: Vec<Message>,
    /// Initial/snapshot state.
    pub state: Value,
    /// Patches applied since the last snapshot.
    pub patches: Vec<TrackedPatch>,
    /// Metadata.
    #[serde(default)]
    pub metadata: SessionMetadata,
}

/// Session metadata.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SessionMetadata {
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

impl Session {
    /// Create a new session with the given ID.
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            messages: Vec::new(),
            state: Value::Object(serde_json::Map::new()),
            patches: Vec::new(),
            metadata: SessionMetadata::default(),
        }
    }

    /// Create a new session with initial state.
    pub fn with_initial_state(id: impl Into<String>, state: Value) -> Self {
        Self {
            id: id.into(),
            messages: Vec::new(),
            state,
            patches: Vec::new(),
            metadata: SessionMetadata::default(),
        }
    }

    /// Add a message to the session (pure function, returns new Session).
    #[must_use]
    pub fn with_message(mut self, msg: Message) -> Self {
        self.messages.push(msg);
        self
    }

    /// Add multiple messages (pure function, returns new Session).
    #[must_use]
    pub fn with_messages(mut self, msgs: impl IntoIterator<Item = Message>) -> Self {
        self.messages.extend(msgs);
        self
    }

    /// Add a patch to the session (pure function, returns new Session).
    #[must_use]
    pub fn with_patch(mut self, patch: TrackedPatch) -> Self {
        self.patches.push(patch);
        self
    }

    /// Add multiple patches (pure function, returns new Session).
    #[must_use]
    pub fn with_patches(mut self, patches: impl IntoIterator<Item = TrackedPatch>) -> Self {
        self.patches.extend(patches);
        self
    }

    /// Rebuild the current state by applying all patches to the base state.
    ///
    /// This is a pure function - it doesn't modify the session.
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
    /// Returns a new Session with the current state as base and empty patches.
    pub fn snapshot(self) -> CarveResult<Self> {
        let current_state = self.rebuild_state()?;
        Ok(Self {
            id: self.id,
            messages: self.messages,
            state: current_state,
            patches: Vec::new(),
            metadata: self.metadata,
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
    fn test_session_new() {
        let session = Session::new("test-1");
        assert_eq!(session.id, "test-1");
        assert!(session.messages.is_empty());
        assert!(session.patches.is_empty());
    }

    #[test]
    fn test_session_with_initial_state() {
        let state = json!({"counter": 0});
        let session = Session::with_initial_state("test-1", state.clone());
        assert_eq!(session.state, state);
    }

    #[test]
    fn test_session_with_message() {
        let session = Session::new("test-1")
            .with_message(Message::user("Hello"))
            .with_message(Message::assistant("Hi!"));

        assert_eq!(session.message_count(), 2);
        assert_eq!(session.messages[0].content, "Hello");
        assert_eq!(session.messages[1].content, "Hi!");
    }

    #[test]
    fn test_session_with_patch() {
        let session = Session::new("test-1");
        let patch = TrackedPatch::new(Patch::new().with_op(Op::set(path!("a"), json!(1))));

        let session = session.with_patch(patch);
        assert_eq!(session.patch_count(), 1);
    }

    #[test]
    fn test_session_rebuild_state_empty() {
        let state = json!({"counter": 0});
        let session = Session::with_initial_state("test-1", state.clone());

        let rebuilt = session.rebuild_state().unwrap();
        assert_eq!(rebuilt, state);
    }

    #[test]
    fn test_session_rebuild_state_with_patches() {
        let state = json!({"counter": 0});
        let session = Session::with_initial_state("test-1", state)
            .with_patch(TrackedPatch::new(
                Patch::new().with_op(Op::set(path!("counter"), json!(1))),
            ))
            .with_patch(TrackedPatch::new(
                Patch::new().with_op(Op::set(path!("name"), json!("test"))),
            ));

        let rebuilt = session.rebuild_state().unwrap();
        assert_eq!(rebuilt["counter"], 1);
        assert_eq!(rebuilt["name"], "test");
    }

    #[test]
    fn test_session_snapshot() {
        let state = json!({"counter": 0});
        let session = Session::with_initial_state("test-1", state)
            .with_patch(TrackedPatch::new(
                Patch::new().with_op(Op::set(path!("counter"), json!(5))),
            ));

        assert_eq!(session.patch_count(), 1);

        let snapshotted = session.snapshot().unwrap();
        assert_eq!(snapshotted.patch_count(), 0);
        assert_eq!(snapshotted.state["counter"], 5);
    }

    #[test]
    fn test_session_needs_snapshot() {
        let session = Session::new("test-1");
        assert!(!session.needs_snapshot(10));

        let session = (0..10).fold(session, |s, i| {
            s.with_patch(TrackedPatch::new(
                Patch::new().with_op(Op::set(path!("field").key(&i.to_string()), json!(i))),
            ))
        });

        assert!(session.needs_snapshot(10));
        assert!(!session.needs_snapshot(20));
    }

    #[test]
    fn test_session_serialization() {
        let session = Session::new("test-1")
            .with_message(Message::user("Hello"));

        let json = serde_json::to_string(&session).unwrap();
        let restored: Session = serde_json::from_str(&json).unwrap();

        assert_eq!(restored.id, "test-1");
        assert_eq!(restored.message_count(), 1);
    }

    #[test]
    fn test_session_replay_to() {
        let state = json!({"counter": 0});
        let session = Session::with_initial_state("test-1", state)
            .with_patch(TrackedPatch::new(
                Patch::new().with_op(Op::set(path!("counter"), json!(10))),
            ))
            .with_patch(TrackedPatch::new(
                Patch::new().with_op(Op::set(path!("counter"), json!(20))),
            ))
            .with_patch(TrackedPatch::new(
                Patch::new().with_op(Op::set(path!("counter"), json!(30))),
            ));

        // Replay to patch 0 (after first patch)
        let state_at_0 = session.replay_to(0).unwrap();
        assert_eq!(state_at_0["counter"], 10);

        // Replay to patch 1 (after second patch)
        let state_at_1 = session.replay_to(1).unwrap();
        assert_eq!(state_at_1["counter"], 20);

        // Replay to patch 2 (after third patch)
        let state_at_2 = session.replay_to(2).unwrap();
        assert_eq!(state_at_2["counter"], 30);

        // Replay beyond available patches returns full state
        let state_beyond = session.replay_to(100).unwrap();
        assert_eq!(state_beyond["counter"], 30);
    }

    #[test]
    fn test_session_replay_to_empty() {
        let state = json!({"counter": 0});
        let session = Session::with_initial_state("test-1", state.clone());

        // Replay on empty patches returns base state
        let replayed = session.replay_to(0).unwrap();
        assert_eq!(replayed, state);
    }
}
