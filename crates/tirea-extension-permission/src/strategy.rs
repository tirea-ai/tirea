use crate::model::{
    PermissionEvaluation, PermissionRuleset, PermissionSubject, ToolPermissionBehavior,
};
use crate::state::permission_rules_from_snapshot;
use serde_json::Value;

/// Evaluate permission rules for a tool call with arguments.
#[must_use]
pub fn evaluate_tool_permission(
    ruleset: &PermissionRuleset,
    tool_id: &str,
    tool_args: &Value,
) -> PermissionEvaluation {
    let subject = PermissionSubject::tool(tool_id);
    let matched_rule = ruleset.rule_for_tool_call(tool_id, tool_args).cloned();
    let behavior = matched_rule
        .as_ref()
        .map_or(ruleset.default_behavior, |rule| rule.behavior);

    PermissionEvaluation {
        subject,
        behavior,
        matched_rule,
    }
}

/// Resolve effective permission behavior from a state snapshot.
#[must_use]
pub fn resolve_permission_behavior(
    snapshot: &Value,
    tool_id: &str,
    tool_args: &Value,
) -> ToolPermissionBehavior {
    let ruleset = permission_rules_from_snapshot(snapshot);
    evaluate_tool_permission(&ruleset, tool_id, tool_args).behavior
}
