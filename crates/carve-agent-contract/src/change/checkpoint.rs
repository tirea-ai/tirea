use crate::change::{AgentChangeSet, CheckpointReason};
use crate::conversation::Message;
use carve_state::TrackedPatch;
use serde_json::Value;
use std::sync::Arc;

/// Checkpoint payload generated from runtime `AgentState`.
#[derive(Debug, Clone)]
pub struct CheckpointChangeSet {
    /// Storage version expected by this change set.
    pub expected_version: u64,
    /// Incremental delta payload for persistence.
    pub delta: AgentChangeSet,
}

impl CheckpointChangeSet {
    /// Build a `CheckpointChangeSet` from explicit delta components.
    pub fn from_parts(
        expected_version: u64,
        run_id: impl Into<String>,
        parent_run_id: Option<String>,
        reason: CheckpointReason,
        messages: Vec<Arc<Message>>,
        patches: Vec<TrackedPatch>,
        snapshot: Option<Value>,
    ) -> Self {
        Self {
            expected_version,
            delta: AgentChangeSet {
                run_id: run_id.into(),
                parent_run_id,
                reason,
                messages,
                patches,
                snapshot,
            },
        }
    }
}
