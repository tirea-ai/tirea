use std::sync::Arc;

use awaken_contract::StateMap;
use awaken_contract::model::{Phase, ScheduledActionSpec};
use awaken_contract::state::Snapshot;
use awaken_runtime::agent::state::ExcludeTool;
use awaken_runtime::{PhaseContext, PhaseHook};

use crate::rules::{PermissionRule, ToolPermissionBehavior};
use crate::state::{PermissionPolicy, PermissionPolicyKey};

use super::filter::PermissionToolFilterHook;

fn snapshot_with_policy(policy: PermissionPolicy) -> Snapshot {
    let mut state_map = StateMap::default();
    state_map.insert::<PermissionPolicyKey>(policy);
    Snapshot::new(0, Arc::new(state_map))
}

fn make_ctx(snapshot: Snapshot) -> PhaseContext {
    PhaseContext::new(Phase::BeforeInference, snapshot)
}

#[tokio::test]
async fn no_rules_yields_no_exclusions() {
    let ctx = make_ctx(snapshot_with_policy(PermissionPolicy::default()));
    let cmd = PermissionToolFilterHook.run(&ctx).await.unwrap();
    assert!(cmd.scheduled_actions().is_empty());
}

#[tokio::test]
async fn unconditional_deny_excludes_tool() {
    let mut policy = PermissionPolicy::default();
    let rule = PermissionRule::new_tool("dangerous_tool", ToolPermissionBehavior::Deny);
    policy.rules.insert(rule.subject.key(), rule);

    let ctx = make_ctx(snapshot_with_policy(policy));
    let cmd = PermissionToolFilterHook.run(&ctx).await.unwrap();

    assert_eq!(cmd.scheduled_actions().len(), 1);
    assert_eq!(cmd.scheduled_actions()[0].key, ExcludeTool::KEY);
    let payload: String =
        ExcludeTool::decode_payload(cmd.scheduled_actions()[0].payload.clone()).unwrap();
    assert_eq!(payload, "dangerous_tool");
}

#[tokio::test]
async fn multiple_unconditional_denies_produce_multiple_exclusions() {
    let mut policy = PermissionPolicy::default();
    for name in ["rm", "shutdown", "drop_table"] {
        let rule = PermissionRule::new_tool(name, ToolPermissionBehavior::Deny);
        policy.rules.insert(rule.subject.key(), rule);
    }

    let ctx = make_ctx(snapshot_with_policy(policy));
    let cmd = PermissionToolFilterHook.run(&ctx).await.unwrap();

    let mut excluded: Vec<String> = cmd
        .scheduled_actions()
        .iter()
        .map(|a| ExcludeTool::decode_payload(a.payload.clone()).unwrap())
        .collect();
    excluded.sort();
    assert_eq!(excluded, vec!["drop_table", "rm", "shutdown"]);
}

#[tokio::test]
async fn allow_rule_does_not_exclude() {
    let mut policy = PermissionPolicy::default();
    let rule = PermissionRule::new_tool("safe_tool", ToolPermissionBehavior::Allow);
    policy.rules.insert(rule.subject.key(), rule);

    let ctx = make_ctx(snapshot_with_policy(policy));
    let cmd = PermissionToolFilterHook.run(&ctx).await.unwrap();
    assert!(cmd.scheduled_actions().is_empty());
}

#[tokio::test]
async fn ask_rule_does_not_exclude() {
    let mut policy = PermissionPolicy::default();
    let rule = PermissionRule::new_tool("ask_tool", ToolPermissionBehavior::Ask);
    policy.rules.insert(rule.subject.key(), rule);

    let ctx = make_ctx(snapshot_with_policy(policy));
    let cmd = PermissionToolFilterHook.run(&ctx).await.unwrap();
    assert!(cmd.scheduled_actions().is_empty());
}

#[tokio::test]
async fn conditional_deny_does_not_exclude() {
    use awaken_tool_pattern::ToolCallPattern;

    let mut policy = PermissionPolicy::default();
    // Pattern with argument condition: `Edit(file_path ~ "/etc/*")`
    let pattern = ToolCallPattern::tool_with_primary("Edit", "/etc/*");
    let rule = PermissionRule::new_pattern(pattern, ToolPermissionBehavior::Deny);
    policy.rules.insert(rule.subject.key(), rule);

    let ctx = make_ctx(snapshot_with_policy(policy));
    let cmd = PermissionToolFilterHook.run(&ctx).await.unwrap();
    // Conditional deny should NOT be excluded at BeforeInference
    assert!(cmd.scheduled_actions().is_empty());
}

#[tokio::test]
async fn empty_state_yields_no_exclusions() {
    // No policy in state at all
    let snapshot = Snapshot::new(0, Arc::new(StateMap::default()));
    let ctx = make_ctx(snapshot);
    let cmd = PermissionToolFilterHook.run(&ctx).await.unwrap();
    assert!(cmd.scheduled_actions().is_empty());
}

#[tokio::test]
async fn mixed_rules_only_excludes_unconditional_denies() {
    use awaken_tool_pattern::ToolCallPattern;

    let mut policy = PermissionPolicy::default();

    // Unconditional deny
    let rule = PermissionRule::new_tool("rm", ToolPermissionBehavior::Deny);
    policy.rules.insert(rule.subject.key(), rule);

    // Allow
    let rule = PermissionRule::new_tool("read", ToolPermissionBehavior::Allow);
    policy.rules.insert(rule.subject.key(), rule);

    // Ask
    let rule = PermissionRule::new_tool("write", ToolPermissionBehavior::Ask);
    policy.rules.insert(rule.subject.key(), rule);

    // Conditional deny
    let pattern = ToolCallPattern::tool_with_primary("Bash", "rm *");
    let rule = PermissionRule::new_pattern(pattern, ToolPermissionBehavior::Deny);
    policy.rules.insert(rule.subject.key(), rule);

    let ctx = make_ctx(snapshot_with_policy(policy));
    let cmd = PermissionToolFilterHook.run(&ctx).await.unwrap();

    // Only "rm" should be excluded
    assert_eq!(cmd.scheduled_actions().len(), 1);
    let payload: String =
        ExcludeTool::decode_payload(cmd.scheduled_actions()[0].payload.clone()).unwrap();
    assert_eq!(payload, "rm");
}
