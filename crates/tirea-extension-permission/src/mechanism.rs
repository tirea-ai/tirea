use crate::actions::{deny_missing_call_id, deny_tool, request_permission};
use crate::form::permission_confirmation_ticket;
use crate::model::{PermissionEvaluation, ToolPermissionBehavior};
use tirea_contract::io::ResumeDecisionAction;
use tirea_contract::runtime::phase::BeforeToolExecuteAction;

/// Runtime input required to enforce permission decisions for one tool call.
pub struct PermissionMechanismInput<'a> {
    pub tool_id: &'a str,
    pub tool_args: serde_json::Value,
    pub call_id: Option<&'a str>,
    pub resume_action: Option<ResumeDecisionAction>,
}

/// Mechanism output after combining strategy verdict with runtime state.
pub enum PermissionMechanismDecision {
    Proceed,
    Action(BeforeToolExecuteAction),
}

/// Apply runtime permission mechanism to a strategy verdict.
#[must_use]
pub fn enforce_permission(
    input: PermissionMechanismInput<'_>,
    evaluation: &PermissionEvaluation,
) -> PermissionMechanismDecision {
    if input
        .resume_action
        .is_some_and(|action| matches!(action, ResumeDecisionAction::Resume))
    {
        return PermissionMechanismDecision::Proceed;
    }

    match evaluation.behavior {
        ToolPermissionBehavior::Allow => PermissionMechanismDecision::Proceed,
        ToolPermissionBehavior::Deny => {
            PermissionMechanismDecision::Action(deny_tool(input.tool_id))
        }
        ToolPermissionBehavior::Ask => {
            let Some(call_id) = input.call_id.filter(|call_id| !call_id.is_empty()) else {
                return PermissionMechanismDecision::Action(deny_missing_call_id());
            };
            PermissionMechanismDecision::Action(request_permission(permission_confirmation_ticket(
                call_id,
                input.tool_id,
                input.tool_args,
            )))
        }
    }
}
