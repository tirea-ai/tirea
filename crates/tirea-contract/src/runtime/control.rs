//! Runtime control-state schema stored under internal `__*` top-level paths.
//!
//! These types define durable runtime control state for cross-step and cross-run
//! flow control (suspended tool calls, resume decisions, and inference error envelope).

use crate::event::suspension::{FrontendToolInvocation, Suspension};
use crate::runtime::state_paths::{SUSPENDED_TOOL_CALLS_STATE_PATH, TOOL_CALL_STATES_STATE_PATH};
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
/// The core loop only stores `call_id` + generic `Suspension`; it does not
/// interpret the semantics (policy checks, frontend tools, user confirmation).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SuspendedCall {
    pub call_id: String,
    pub tool_name: String,
    pub suspension: Suspension,
    /// Frontend invocation metadata for routing decisions.
    pub invocation: FrontendToolInvocation,
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
/// - suspension id
/// - frontend invocation call id
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ToolCallDecision {
    /// External target identifier used to resolve suspended call.
    pub target_id: String,
    /// Idempotency key for the decision submission.
    pub decision_id: String,
    /// Resume or cancel action.
    pub action: ResumeDecisionAction,
    /// Raw response payload from suspension/frontend.
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub result: Value,
    /// Optional human-readable reason.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// Decision update timestamp (unix millis).
    pub updated_at: u64,
}

impl ToolCallDecision {
    /// Build an explicit resume decision.
    pub fn resume(target_id: impl Into<String>, result: Value, updated_at: u64) -> Self {
        Self {
            target_id: target_id.into(),
            decision_id: String::new(),
            action: ResumeDecisionAction::Resume,
            result,
            reason: None,
            updated_at,
        }
    }

    /// Build an explicit cancel decision.
    pub fn cancel(
        target_id: impl Into<String>,
        result: Value,
        reason: Option<String>,
        updated_at: u64,
    ) -> Self {
        Self {
            target_id: target_id.into(),
            decision_id: String::new(),
            action: ResumeDecisionAction::Cancel,
            result,
            reason,
            updated_at,
        }
    }
}

/// One pending decision waiting to be applied to a suspended tool call.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ResumeDecision {
    /// Idempotency key for the external decision.
    pub decision_id: String,
    /// Resume or cancel action.
    pub action: ResumeDecisionAction,
    /// Raw response payload from suspension frontend.
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

/// Tool call lifecycle status for suspend/resume capable execution.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ToolCallStatus {
    /// Newly observed call that has not started execution yet.
    #[default]
    New,
    /// Call is currently executing.
    Running,
    /// Call is suspended waiting for a resume decision.
    Suspended,
    /// Call is resuming with external decision input.
    Resuming,
    /// Call finished successfully.
    Succeeded,
    /// Call finished with failure.
    Failed,
    /// Call was cancelled.
    Cancelled,
}

/// Resume input payload attached to a suspended tool call.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ToolCallResume {
    /// Idempotency key for the decision submission.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub decision_id: String,
    /// Resume or cancel action.
    pub action: ResumeDecisionAction,
    /// Raw response payload from suspension/frontend.
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub result: Value,
    /// Optional human-readable reason.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// Decision update timestamp (unix millis).
    #[serde(default)]
    pub updated_at: u64,
}

/// Durable per-tool-call runtime state.
///
/// This is run-time state persisted in thread state so tool execution can be
/// resumed as a re-entrant flow (`before_tool_execute -> execute -> after_tool_execute`).
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, State)]
pub struct ToolCallState {
    /// Stable tool call id.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub call_id: String,
    /// Tool name.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub tool_name: String,
    /// Tool arguments snapshot.
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub arguments: Value,
    /// Lifecycle status.
    #[serde(default)]
    pub status: ToolCallStatus,
    /// Token used by external actor to resume this call.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resume_token: Option<String>,
    /// Resume payload written by external decision handling.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resume: Option<ToolCallResume>,
    /// Plugin/tool scratch data for this call.
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub scratch: Value,
    /// Last update timestamp (unix millis).
    #[serde(default)]
    pub updated_at: u64,
}

/// Durable per-call runtime map persisted at `state["__tool_call_states"]`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, State)]
#[tirea(path = "__tool_call_states")]
pub struct ToolCallStatesState {
    /// Runtime state keyed by `tool_call_id`.
    #[serde(default)]
    #[tirea(default = "HashMap::new()")]
    pub calls: HashMap<String, ToolCallState>,
}

/// Durable inference-error envelope persisted at `state["__inference_error"]`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, State)]
#[tirea(path = "__inference_error")]
pub struct InferenceErrorState {
    #[tirea(default = "None")]
    pub error: Option<InferenceError>,
}

/// Parse suspended tool calls from a rebuilt state snapshot.
pub fn suspended_calls_from_state(state: &Value) -> HashMap<String, SuspendedCall> {
    state
        .get(SUSPENDED_TOOL_CALLS_STATE_PATH)
        .and_then(|value| value.get("calls"))
        .cloned()
        .and_then(|value| serde_json::from_value(value).ok())
        .unwrap_or_default()
}

/// Parse persisted tool call runtime states from a rebuilt state snapshot.
pub fn tool_call_states_from_state(state: &Value) -> HashMap<String, ToolCallState> {
    state
        .get(TOOL_CALL_STATES_STATE_PATH)
        .and_then(|value| value.get("calls"))
        .cloned()
        .and_then(|value| serde_json::from_value(value).ok())
        .unwrap_or_default()
}

fn first_suspended_call(calls: &HashMap<String, SuspendedCall>) -> Option<&SuspendedCall> {
    calls
        .iter()
        .min_by(|(left, _), (right, _)| left.cmp(right))
        .map(|(_, call)| call)
}

/// Read the first suspension from suspended call state.
pub fn first_suspension_from_state(state: &Value) -> Option<Suspension> {
    let calls = suspended_calls_from_state(state);
    first_suspended_call(&calls).map(|call| call.suspension.clone())
}

/// Read the first suspended invocation from suspended call state.
pub fn first_suspended_invocation_from_state(state: &Value) -> Option<FrontendToolInvocation> {
    let calls = suspended_calls_from_state(state);
    first_suspended_call(&calls).map(|call| call.invocation.clone())
}

/// Helpers for reading suspended-call state from `Thread`.
pub trait SuspendedCallsExt {
    /// Read the first suspension from durable control state.
    fn first_suspension(&self) -> Option<Suspension>;
}

impl SuspendedCallsExt for Thread {
    fn first_suspension(&self) -> Option<Suspension> {
        self.rebuild_state()
            .ok()
            .and_then(|state| first_suspension_from_state(&state))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::suspension::SuspensionResponse;
    use serde_json::Value;

    #[test]
    fn test_suspension_new() {
        let suspension = Suspension::new("int_1", "confirm");
        assert_eq!(suspension.id, "int_1");
        assert_eq!(suspension.action, "confirm");
        assert!(suspension.message.is_empty());
        assert_eq!(suspension.parameters, Value::Null);
        assert!(suspension.response_schema.is_none());
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
    fn tool_call_decision_resume_constructor_sets_resume_action() {
        let mut decision = ToolCallDecision::resume("fc_1", Value::Bool(true), 123);
        decision.decision_id = "decision_fc_1".to_string();
        assert_eq!(decision.target_id, "fc_1");
        assert_eq!(decision.decision_id, "decision_fc_1");
        assert!(matches!(decision.action, ResumeDecisionAction::Resume));
        assert_eq!(decision.result, Value::Bool(true));
        assert!(decision.reason.is_none());
        assert_eq!(decision.updated_at, 123);
    }

    #[test]
    fn tool_call_decision_cancel_constructor_sets_cancel_action() {
        let mut decision = ToolCallDecision::cancel(
            "fc_2",
            SuspensionResponse::new(
                "fc_2",
                serde_json::json!({
                    "approved": false,
                    "reason": "denied by user"
                }),
            )
            .result,
            Some("denied by user".to_string()),
            456,
        );
        decision.decision_id = "decision_fc_2".to_string();
        assert!(matches!(decision.action, ResumeDecisionAction::Cancel));
        assert_eq!(decision.reason.as_deref(), Some("denied by user"));
        assert_eq!(decision.updated_at, 456);
    }
}
