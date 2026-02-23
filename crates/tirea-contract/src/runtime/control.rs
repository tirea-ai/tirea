//! Runtime control-state schema stored under internal `__*` top-level paths.
//!
//! These types define durable runtime control state for cross-step and cross-run
//! flow control (suspended tool calls, resume decisions, and inference error envelope).

use crate::event::interaction::{FrontendToolInvocation, Interaction, InteractionResponse};
use crate::thread::Thread;
use serde::{Deserialize, Serialize};
use serde_json::Value;
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

/// Action to apply for a suspended tool call.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ResumeDecisionAction {
    Resume,
    Cancel,
}

/// External decision command routed to a suspended tool call.
///
/// `target_id` may refer to:
/// - suspended `call_id`
/// - interaction id
/// - frontend invocation call id
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ToolCallDecision {
    /// External target identifier used to resolve suspended call.
    pub target_id: String,
    /// Idempotency key for the decision submission.
    pub decision_id: String,
    /// Resume or cancel action.
    pub action: ResumeDecisionAction,
    /// Raw response payload from interaction/frontend.
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub result: Value,
    /// Optional human-readable reason.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// Decision update timestamp (unix millis).
    pub updated_at: u64,
}

impl ToolCallDecision {
    /// Build a decision from a legacy interaction response payload.
    pub fn from_interaction_response(response: InteractionResponse, updated_at: u64) -> Self {
        let action = if InteractionResponse::is_denied(&response.result) {
            ResumeDecisionAction::Cancel
        } else {
            ResumeDecisionAction::Resume
        };
        let reason = if matches!(action, ResumeDecisionAction::Cancel) {
            response
                .result
                .as_object()
                .and_then(|obj| {
                    obj.get("reason")
                        .and_then(Value::as_str)
                        .or_else(|| obj.get("message").and_then(Value::as_str))
                })
                .map(str::to_string)
        } else {
            None
        };
        Self {
            target_id: response.interaction_id.clone(),
            decision_id: format!("decision_{}", response.interaction_id),
            action,
            result: response.result,
            reason,
            updated_at,
        }
    }
}

impl From<InteractionResponse> for ToolCallDecision {
    fn from(response: InteractionResponse) -> Self {
        let updated_at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_millis().min(u128::from(u64::MAX)) as u64);
        Self::from_interaction_response(response, updated_at)
    }
}

/// One pending decision waiting to be applied to a suspended tool call.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ResumeDecision {
    /// Idempotency key for the external decision.
    pub decision_id: String,
    /// Resume or cancel action.
    pub action: ResumeDecisionAction,
    /// Raw response payload from interaction frontend.
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub result: Value,
    /// Optional human-readable reason.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// Last write timestamp (unix millis).
    pub updated_at: u64,
}

/// Durable rendezvous for resume/cancel decisions keyed by `call_id`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, State)]
#[tirea(path = "__resume_decisions")]
pub struct ResumeDecisionsState {
    /// Pending decisions to apply.
    #[serde(default)]
    #[tirea(default = "HashMap::new()")]
    pub calls: HashMap<String, ResumeDecision>,
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

        let resume_decisions = ResumeDecisionsState::default();
        assert!(resume_decisions.calls.is_empty());

        let err = InferenceErrorState::default();
        assert!(err.error.is_none());
    }

    #[test]
    fn tool_call_decision_from_interaction_response_resume() {
        let response = InteractionResponse::new("fc_1", Value::Bool(true));
        let decision = ToolCallDecision::from_interaction_response(response, 123);
        assert_eq!(decision.target_id, "fc_1");
        assert_eq!(decision.decision_id, "decision_fc_1");
        assert!(matches!(decision.action, ResumeDecisionAction::Resume));
        assert_eq!(decision.result, Value::Bool(true));
        assert!(decision.reason.is_none());
        assert_eq!(decision.updated_at, 123);
    }

    #[test]
    fn tool_call_decision_from_interaction_response_cancel_with_reason() {
        let response = InteractionResponse::new(
            "fc_2",
            serde_json::json!({
                "approved": false,
                "reason": "denied by user"
            }),
        );
        let decision = ToolCallDecision::from_interaction_response(response, 456);
        assert!(matches!(decision.action, ResumeDecisionAction::Cancel));
        assert_eq!(decision.reason.as_deref(), Some("denied by user"));
        assert_eq!(decision.updated_at, 456);
    }
}
