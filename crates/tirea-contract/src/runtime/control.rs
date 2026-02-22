//! Runtime control-state schema stored under internal `__*` top-level paths.
//!
//! These types define durable runtime control state for cross-step and cross-run
//! flow control (suspended tool calls, resume queue, suspension resolutions, and
//! inference error envelope).

use crate::event::interaction::{FrontendToolInvocation, Interaction, InteractionResponse};
use crate::thread::Thread;
use crate::thread::ToolCall;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tirea_state::State;

/// Inference error emitted by the loop and consumed by telemetry plugins.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct InferenceError {
    /// Stable error class used for metrics/telemetry dimensions.
    #[serde(rename = "type")]
    pub error_type: String,
    /// Human-readable error message.
    pub message: String,
}

/// A tool call that has been suspended, awaiting external resolution.
///
/// The core loop only stores `call_id` + generic `Interaction`; it does not
/// interpret the semantics (permissions, frontend tools, user confirmation).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SuspendedCall {
    pub call_id: String,
    pub tool_name: String,
    pub interaction: Interaction,
    /// Optional frontend invocation metadata for routing decisions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frontend_invocation: Option<FrontendToolInvocation>,
}

/// Durable suspended tool-call map persisted at `state["__suspended_tool_calls"]`.
///
/// This is the only long-lived control state required to recover pending tool
/// calls across runs.
#[derive(Debug, Clone, Default, Serialize, Deserialize, State)]
#[tirea(path = "__suspended_tool_calls")]
pub struct SuspendedToolCallsState {
    /// Per-call suspended tool calls awaiting external resolution.
    #[serde(default)]
    #[tirea(default = "HashMap::new()")]
    pub calls: HashMap<String, SuspendedCall>,
}

/// Durable resume queue persisted at `state["__resume_tool_calls"]`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, State)]
#[tirea(path = "__resume_tool_calls")]
pub struct ResumeToolCallsState {
    /// Tool calls queued for resume/replay.
    #[serde(default)]
    #[tirea(default = "Vec::new()")]
    pub calls: Vec<ToolCall>,
}

/// Durable suspension decision records persisted at `state["__resolved_suspensions"]`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, State)]
#[tirea(path = "__resolved_suspensions")]
pub struct ResolvedSuspensionsState {
    /// Resolved suspension decisions (approve/deny payloads).
    #[serde(default)]
    #[tirea(default = "Vec::new()")]
    pub resolutions: Vec<InteractionResponse>,
}

/// Durable inference-error envelope persisted at `state["__inference_error"]`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, State)]
#[tirea(path = "__inference_error")]
pub struct InferenceErrorState {
    #[tirea(default = "None")]
    pub error: Option<InferenceError>,
}

/// Helpers for accessing loop control state from `Thread`.
pub trait LoopControlExt {
    /// Read pending interaction from durable control state.
    fn pending_interaction(&self) -> Option<Interaction>;
}

impl LoopControlExt for Thread {
    fn pending_interaction(&self) -> Option<Interaction> {
        self.rebuild_state().ok().and_then(|state| {
            let calls = state
                .get(SuspendedToolCallsState::PATH)
                .and_then(|v| v.get("calls"))
                .cloned()
                .and_then(|v| serde_json::from_value::<HashMap<String, SuspendedCall>>(v).ok())
                .unwrap_or_default();
            calls
                .iter()
                .min_by(|(left, _), (right, _)| left.cmp(right))
                .map(|(_, call)| call.interaction.clone())
        })
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
        let suspended = SuspendedToolCallsState::default();
        assert!(suspended.calls.is_empty());

        let resume = ResumeToolCallsState::default();
        assert!(resume.calls.is_empty());

        let resolved = ResolvedSuspensionsState::default();
        assert!(resolved.resolutions.is_empty());

        let err = InferenceErrorState::default();
        assert!(err.error.is_none());
    }
}
