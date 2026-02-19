//! Delegation types for sub-agent run orchestration.
//!
//! These types model the persisted state of delegated agent runs
//! (created by `agent_run` / `agent_stop` tools).

use crate::contracts::state::AgentState;
use carve_state::State;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Status of a delegated agent run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DelegationStatus {
    Running,
    Completed,
    Failed,
    Stopped,
}

impl DelegationStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Stopped => "stopped",
        }
    }
}

/// Persisted record for a delegated sub-agent run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DelegationRecord {
    /// Stable run id returned to the caller.
    pub run_id: String,
    /// Parent caller run id (from caller runtime `run_id`), if available.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_run_id: Option<String>,
    /// Target agent id delegated by `agent_run`.
    pub target_agent_id: String,
    /// Current run status.
    pub status: DelegationStatus,
    /// Last assistant message from the child agent (if available).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assistant: Option<String>,
    /// Error message (if the run failed or was force-stopped).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Last known child session snapshot for resume/recovery.
    #[serde(default, rename = "thread", skip_serializing_if = "Option::is_none")]
    pub agent_state: Option<AgentState>,
}

/// Persisted sub-agent delegation state at `state["agent_runs"]`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, State)]
#[carve(path = "agent_runs")]
pub struct DelegationState {
    /// Delegated runs keyed by `run_id`.
    #[carve(default = "HashMap::new()")]
    pub runs: HashMap<String, DelegationRecord>,
}
