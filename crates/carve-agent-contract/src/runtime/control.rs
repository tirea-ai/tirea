//! Loop control-state schema stored under `AgentState.state["loop_control"]`.
//!
//! These types define durable loop-control state for cross-step and cross-run
//! flow control (pending interactions, inference error envelope).

use crate::runtime::interaction::Interaction;
use crate::state::AgentState;
use carve_state::State;
use serde::{Deserialize, Serialize};

/// JSON path under `AgentState.state` where loop control data is stored.
pub const LOOP_CONTROL_STATE_PATH: &str = "loop_control";

/// Inference error emitted by the loop and consumed by telemetry plugins.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct InferenceError {
    /// Stable error class used for metrics/telemetry dimensions.
    #[serde(rename = "type")]
    pub error_type: String,
    /// Human-readable error message.
    pub message: String,
}

/// Durable loop control state persisted at `state["loop_control"]`.
///
/// Used for cross-step and cross-run flow control that must survive restarts
/// (not ephemeral in-memory variables).
#[derive(Debug, Clone, Default, Serialize, Deserialize, State)]
pub struct LoopControlState {
    /// Pending interaction that must be resolved by the client before the run can continue.
    #[carve(default = "None")]
    pub pending_interaction: Option<Interaction>,
    /// Inference error envelope for AfterInference cleanup flow.
    #[carve(default = "None")]
    pub inference_error: Option<InferenceError>,
}

/// Helpers for accessing loop control state from `AgentState`.
pub trait LoopControlExt {
    /// Typed accessor for durable loop control substate at `state["loop_control"]`.
    fn loop_control(&self) -> <LoopControlState as carve_state::State>::Ref<'_>;

    /// Read pending interaction from durable control state.
    fn pending_interaction(&self) -> Option<Interaction>;

    /// Write pending interaction into durable control state.
    fn set_pending_interaction(&self, interaction: Option<Interaction>);
}

impl LoopControlExt for AgentState {
    fn loop_control(&self) -> <LoopControlState as carve_state::State>::Ref<'_> {
        self.state::<LoopControlState>(LOOP_CONTROL_STATE_PATH)
    }

    fn pending_interaction(&self) -> Option<Interaction> {
        if self.patches.is_empty() {
            return self.loop_control().pending_interaction().ok().flatten();
        }

        self.rebuild_state()
            .ok()
            .and_then(|state| {
                state
                    .get(LOOP_CONTROL_STATE_PATH)
                    .and_then(|rt| rt.get("pending_interaction"))
                    .cloned()
            })
            .and_then(|value| serde_json::from_value::<Interaction>(value).ok())
            .or_else(|| self.loop_control().pending_interaction().ok().flatten())
    }

    fn set_pending_interaction(&self, interaction: Option<Interaction>) {
        self.loop_control().set_pending_interaction(interaction);
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
    fn test_loop_control_state_defaults() {
        let state = LoopControlState::default();
        assert!(state.pending_interaction.is_none());
        assert!(state.inference_error.is_none());
    }
}
