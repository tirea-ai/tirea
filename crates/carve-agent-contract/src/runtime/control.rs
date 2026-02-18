//! Runtime control-state schema stored under `AgentState.state["runtime"]`.
//!
//! These types define durable run-control state shared across runtime, tools,
//! and plugins (pending interactions, agent-run status, outbox buffers, etc.).

use crate::runtime::interaction::{Interaction, InteractionResponse};
use crate::state::{AgentState, ToolCall};
use carve_state_derive::State;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// JSON path under `AgentState.state` where runtime control data is stored.
pub const RUNTIME_CONTROL_STATE_PATH: &str = "runtime";

/// JSON path under `AgentState.state` where agent runs data is stored.
pub const AGENT_RUNS_STATE_PATH: &str = "agent_runs";

/// Interaction action used for agent run recovery confirmation.
pub const AGENT_RECOVERY_INTERACTION_ACTION: &str = "recover_agent_run";

/// Interaction ID prefix used for agent run recovery confirmation.
pub const AGENT_RECOVERY_INTERACTION_PREFIX: &str = "agent_recovery_";

/// Status of a run entry persisted in [`RuntimeControlState`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RunStatus {
    Running,
    Completed,
    Failed,
    Stopped,
}

impl RunStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Stopped => "stopped",
        }
    }
}

/// Persisted snapshot for an `agent_run` execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunState {
    /// Stable run id returned to the caller.
    pub run_id: String,
    /// Parent caller run id (from caller runtime `run_id`), if available.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_run_id: Option<String>,
    /// Target agent id delegated by `agent_run`.
    pub target_agent_id: String,
    /// Current run status.
    pub status: RunStatus,
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

/// Inference error emitted by the loop and consumed by telemetry plugins.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct InferenceError {
    /// Stable error class used for metrics/telemetry dimensions.
    #[serde(rename = "type")]
    pub error_type: String,
    /// Human-readable error message.
    pub message: String,
}

/// Durable runtime control state persisted at `state["runtime"]`.
///
/// Used for cross-step and cross-run flow control that must survive restarts
/// (not ephemeral in-memory variables).
#[derive(Debug, Clone, Default, Serialize, Deserialize, State)]
pub struct RuntimeControlState {
    /// Pending interaction that must be resolved by the client before the run can continue.
    #[carve(default = "None")]
    pub pending_interaction: Option<Interaction>,
    /// Replay queue for tool calls that should run at session start.
    #[carve(default = "Vec::new()")]
    pub replay_tool_calls: Vec<ToolCall>,
    /// Interaction responses that should be emitted as runtime events.
    ///
    /// These are produced by interaction-handling plugins and drained by the runtime.
    #[carve(default = "Vec::new()")]
    pub interaction_resolutions: Vec<InteractionResponse>,
    /// Inference error envelope for AfterInference cleanup flow.
    #[carve(default = "None")]
    pub inference_error: Option<InferenceError>,
}

/// Persisted sub-agent run orchestration state at `state["agent_runs"]`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, State)]
pub struct AgentRunsState {
    /// Sub-agent runs keyed by `run_id`.
    #[carve(default = "HashMap::new()")]
    pub runs: HashMap<String, RunState>,
}

/// Runtime-only helpers for accessing runtime control state from `AgentState`.
pub trait RuntimeControlExt {
    /// Typed accessor for durable runtime control substate at `state["runtime"]`.
    fn runtime_control(&self) -> <RuntimeControlState as carve_state::State>::Ref<'_>;

    /// Read pending interaction from durable control state.
    fn pending_interaction(&self) -> Option<Interaction>;

    /// Write pending interaction into durable control state.
    fn set_pending_interaction(&self, interaction: Option<Interaction>);
}

impl RuntimeControlExt for AgentState {
    fn runtime_control(&self) -> <RuntimeControlState as carve_state::State>::Ref<'_> {
        self.state::<RuntimeControlState>(RUNTIME_CONTROL_STATE_PATH)
    }

    fn pending_interaction(&self) -> Option<Interaction> {
        if self.patches.is_empty() {
            return self
                .runtime_control()
                .pending_interaction()
                .ok()
                .flatten();
        }

        self.rebuild_state()
            .ok()
            .and_then(|state| {
                state
                    .get(RUNTIME_CONTROL_STATE_PATH)
                    .and_then(|rt| rt.get("pending_interaction"))
                    .cloned()
            })
            .and_then(|value| serde_json::from_value::<Interaction>(value).ok())
            .or_else(|| {
                self.runtime_control()
                    .pending_interaction()
                    .ok()
                    .flatten()
            })
    }

    fn set_pending_interaction(&self, interaction: Option<Interaction>) {
        self.runtime_control().set_pending_interaction(interaction);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    #[test]
    fn test_interaction_new() {
        let interaction = Interaction::new("int_1", "confirm");
        assert_eq!(interaction.id, "int_1");
        assert_eq!(interaction.action, "confirm");
        assert!(interaction.message.is_empty());
        assert_eq!(interaction.parameters, Value::Null);
        assert!(interaction.response_schema.is_none());
    }

    #[test]
    fn test_run_status_serialization() {
        let status = RunStatus::Stopped;
        let json = serde_json::to_string(&status).unwrap();
        assert_eq!(json, "\"stopped\"");
        assert_eq!(status.as_str(), "stopped");
    }

    #[test]
    fn test_runtime_control_state_defaults() {
        let state = RuntimeControlState::default();
        assert!(state.pending_interaction.is_none());
        assert!(state.replay_tool_calls.is_empty());
        assert!(state.interaction_resolutions.is_empty());
        assert!(state.inference_error.is_none());
    }

    #[test]
    fn test_agent_runs_state_defaults() {
        let state = AgentRunsState::default();
        assert!(state.runs.is_empty());
    }

    #[test]
    fn test_run_state_serialization_with_session() {
        let child =
            AgentState::new("child-1").with_message(crate::state::Message::user("seed"));
        let run = RunState {
            run_id: "run-1".to_string(),
            parent_run_id: Some("parent-run-1".to_string()),
            target_agent_id: "worker".to_string(),
            status: RunStatus::Running,
            assistant: None,
            error: None,
            agent_state: Some(child),
        };
        let json = serde_json::to_value(&run).unwrap();
        assert_eq!(json["run_id"], "run-1");
        assert_eq!(json["parent_run_id"], "parent-run-1");
        assert_eq!(json["target_agent_id"], "worker");
        assert_eq!(json["status"], "running");
        assert_eq!(json["thread"]["id"], "child-1");
    }
}
