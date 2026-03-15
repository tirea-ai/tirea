use crate::model::{
    PermissionEvaluation, PermissionRuleset, PermissionSubject, ToolPermissionBehavior,
};
use crate::state::permission_rules_from_snapshot;

/// Evaluate permission rules for a tool subject.
#[must_use]
pub fn evaluate_tool_permission(
    ruleset: &PermissionRuleset,
    tool_id: &str,
) -> PermissionEvaluation {
    let subject = PermissionSubject::tool(tool_id);
    let matched_rule = ruleset.rule_for_tool(tool_id).cloned();
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
    snapshot: &serde_json::Value,
    tool_id: &str,
) -> ToolPermissionBehavior {
    let ruleset = permission_rules_from_snapshot(snapshot);
    evaluate_tool_permission(&ruleset, tool_id).behavior
}
