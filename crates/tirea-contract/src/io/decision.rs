use crate::runtime::tool_call::lifecycle::ToolCallResume;
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
///
/// The resume payload (decision_id, action, result, reason, updated_at) is
/// shared with `ToolCallResume` via `#[serde(flatten)]`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ToolCallDecision {
    /// External target identifier used to resolve suspended call.
    pub target_id: String,
    /// Resume payload shared with `ToolCallResume`.
    #[serde(flatten)]
    pub resume: ToolCallResume,
}

impl ToolCallDecision {
    /// Build an explicit resume decision.
    pub fn resume(target_id: impl Into<String>, result: Value, updated_at: u64) -> Self {
        Self {
            target_id: target_id.into(),
            resume: ToolCallResume {
                decision_id: String::new(),
                action: ResumeDecisionAction::Resume,
                result,
                reason: None,
                updated_at,
            },
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
            resume: ToolCallResume {
                decision_id: String::new(),
                action: ResumeDecisionAction::Cancel,
                result,
                reason,
                updated_at,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_call_decision_resume_constructor_sets_resume_action() {
        let mut decision = ToolCallDecision::resume("fc_1", Value::Bool(true), 123);
        decision.resume.decision_id = "decision_fc_1".to_string();
        assert_eq!(decision.target_id, "fc_1");
        assert_eq!(decision.resume.decision_id, "decision_fc_1");
        assert!(matches!(decision.resume.action, ResumeDecisionAction::Resume));
        assert_eq!(decision.resume.result, Value::Bool(true));
        assert!(decision.resume.reason.is_none());
        assert_eq!(decision.resume.updated_at, 123);
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
        decision.resume.decision_id = "decision_fc_2".to_string();
        assert!(matches!(decision.resume.action, ResumeDecisionAction::Cancel));
        assert_eq!(decision.resume.reason.as_deref(), Some("denied by user"));
        assert_eq!(decision.resume.updated_at, 456);
    }

    #[test]
    fn tool_call_decision_serde_flatten_roundtrip() {
        let decision = ToolCallDecision::resume("fc_1", Value::Bool(true), 42);
        let json = serde_json::to_value(&decision).unwrap();

        // Flattened: no "resume" key, fields at top level
        assert!(json.get("resume").is_none(), "resume should be flattened");
        assert_eq!(json["target_id"], "fc_1");
        assert_eq!(json["action"], "resume");
        assert_eq!(json["result"], true);

        // Roundtrip
        let deserialized: ToolCallDecision = serde_json::from_value(json).unwrap();
        assert_eq!(deserialized, decision);
    }
}
