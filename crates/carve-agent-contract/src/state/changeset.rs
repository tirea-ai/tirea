//! Shared persistence change-set types shared by runtime and storage.

use crate::state::{AgentState, Message};
use carve_state::TrackedPatch;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Arc;

/// Monotonically increasing version for optimistic concurrency.
pub type Version = u64;

/// Reason for a checkpoint (delta).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum CheckpointReason {
    UserMessage,
    AssistantTurnCommitted,
    ToolResultsCommitted,
    RunFinished,
}

/// An incremental change to a thread produced by a single step.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentChangeSet {
    /// Which run produced this delta.
    pub run_id: String,
    /// Parent run (for sub-agent deltas).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_run_id: Option<String>,
    /// Why this delta was created.
    pub reason: CheckpointReason,
    /// New messages appended in this step.
    pub messages: Vec<Arc<Message>>,
    /// New patches appended in this step.
    pub patches: Vec<TrackedPatch>,
    /// If `Some`, a full state snapshot was taken (replaces base state).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub snapshot: Option<Value>,
}

impl AgentChangeSet {
    /// Build an `AgentChangeSet` from explicit delta components.
    pub fn from_parts(
        run_id: impl Into<String>,
        parent_run_id: Option<String>,
        reason: CheckpointReason,
        messages: Vec<Arc<Message>>,
        patches: Vec<TrackedPatch>,
        snapshot: Option<Value>,
    ) -> Self {
        Self {
            run_id: run_id.into(),
            parent_run_id,
            reason,
            messages,
            patches,
            snapshot,
        }
    }

    /// Apply this delta to a thread in place.
    pub fn apply_to(&self, thread: &mut AgentState) {
        if let Some(ref snapshot) = self.snapshot {
            thread.state = snapshot.clone();
            thread.patches.clear();
        }

        thread.messages.extend(self.messages.iter().cloned());
        thread.patches.extend(self.patches.iter().cloned());
    }
}
