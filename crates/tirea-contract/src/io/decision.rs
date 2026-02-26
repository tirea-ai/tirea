use serde::{Deserialize, Serialize};
use serde_json::Value;

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
/// - pending external tool-call id
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

#[cfg(test)]
mod tests {
    use super::*;

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
            serde_json::json!({
                "approved": false,
                "reason": "denied by user"
            }),
            Some("denied by user".to_string()),
            456,
        );
        decision.decision_id = "decision_fc_2".to_string();
        assert!(matches!(decision.action, ResumeDecisionAction::Cancel));
        assert_eq!(decision.reason.as_deref(), Some("denied by user"));
        assert_eq!(decision.updated_at, 456);
    }
}
