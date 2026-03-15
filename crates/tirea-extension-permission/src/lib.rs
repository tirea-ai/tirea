//! Permission policy extension.
//!
//! External callers use [`PermissionAction`], [`permission_state_action`],
//! [`PermissionPlugin`], and [`ToolPolicyPlugin`].

mod actions;
mod form;
mod mechanism;
mod model;
mod plugin;
mod state;
mod strategy;

pub use actions::{
    apply_tool_policy, deny, deny_missing_call_id, deny_tool, reject_out_of_scope,
    request_permission,
};
pub use form::{permission_confirmation_ticket, PERMISSION_CONFIRM_TOOL_NAME};
pub use mechanism::{enforce_permission, PermissionMechanismDecision, PermissionMechanismInput};
pub use model::{
    PermissionEvaluation, PermissionRule, PermissionRuleScope, PermissionRuleSource,
    PermissionRuleset, PermissionSubject, ToolPermissionBehavior,
};
pub use plugin::{PermissionPlugin, ToolPolicyPlugin, PERMISSION_PLUGIN_ID};
pub use state::{
    permission_rules_from_snapshot, permission_state_action, PermissionAction, PermissionPolicy,
    PermissionPolicyAction,
};
pub use strategy::{evaluate_tool_permission, resolve_permission_behavior};

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tirea_contract::io::ResumeDecisionAction;
    use tirea_contract::runtime::behavior::AgentBehavior;
    use tirea_contract::runtime::phase::Phase;
    use tirea_contract::runtime::phase::{ActionSet, BeforeToolExecuteAction};
    use tirea_contract::runtime::tool_call::ToolCallResume;
    use tirea_contract::RunPolicy;
    use tirea_state::{DocCell, State};

    fn has_block(actions: &ActionSet<BeforeToolExecuteAction>) -> bool {
        actions
            .as_slice()
            .iter()
            .any(|a| matches!(a, BeforeToolExecuteAction::Block(_)))
    }

    fn suspend_action(
        actions: &ActionSet<BeforeToolExecuteAction>,
    ) -> Option<&tirea_contract::runtime::phase::SuspendTicket> {
        actions.as_slice().iter().find_map(|a| match a {
            BeforeToolExecuteAction::Suspend(ticket) => Some(ticket),
            _ => None,
        })
    }

    #[test]
    fn permission_policy_defaults_to_ask_with_no_rules() {
        let state = PermissionPolicy::default();
        assert_eq!(state.default_behavior, ToolPermissionBehavior::Ask);
        assert!(state.rules.is_empty());
    }

    #[test]
    fn permission_state_action_routes_to_policy_state() {
        let action = PermissionAction::SetTool {
            tool_id: "read".to_string(),
            behavior: ToolPermissionBehavior::Allow,
        };
        let serialized = permission_state_action(action).to_serialized_state_action();
        assert_eq!(serialized.base_path, PermissionPolicy::PATH);
        assert!(serialized.payload.is_object());
    }

    #[test]
    fn permission_rules_from_snapshot_reads_new_policy_rules() {
        let snapshot = json!({
            "permission_policy": {
                "default_behavior": "deny",
                "rules": {
                    "tool:read_file": {
                        "subject": { "kind": "tool", "tool_id": "read_file" },
                        "behavior": "allow",
                        "scope": "thread",
                        "source": "runtime"
                    }
                }
            }
        });

        let ruleset = permission_rules_from_snapshot(&snapshot);
        assert_eq!(ruleset.default_behavior, ToolPermissionBehavior::Deny);
        assert_eq!(
            ruleset.rule_for_tool("read_file").map(|rule| rule.behavior),
            Some(ToolPermissionBehavior::Allow)
        );
    }

    #[test]
    fn permission_rules_from_snapshot_reads_legacy_policy_shape() {
        let snapshot = json!({
            "permission_policy": {
                "default_behavior": "ask",
                "allowed_tools": ["read_file"],
                "denied_tools": ["write_file"]
            }
        });

        let ruleset = permission_rules_from_snapshot(&snapshot);
        assert_eq!(
            ruleset.rule_for_tool("read_file").map(|rule| rule.behavior),
            Some(ToolPermissionBehavior::Allow)
        );
        assert_eq!(
            ruleset
                .rule_for_tool("write_file")
                .map(|rule| rule.behavior),
            Some(ToolPermissionBehavior::Deny)
        );
    }

    #[test]
    fn permission_rules_from_snapshot_falls_back_to_legacy_permissions() {
        let snapshot = json!({
            "permissions": {
                "default_behavior": "deny",
                "tools": {
                    "recover_agent_run": "allow"
                }
            }
        });

        let ruleset = permission_rules_from_snapshot(&snapshot);
        assert_eq!(ruleset.default_behavior, ToolPermissionBehavior::Deny);
        assert_eq!(
            ruleset
                .rule_for_tool("recover_agent_run")
                .map(|rule| rule.behavior),
            Some(ToolPermissionBehavior::Allow)
        );
    }

    #[test]
    fn resolve_permission_prefers_new_policy_over_legacy_permissions() {
        let snapshot = json!({
            "permission_policy": {
                "default_behavior": "ask",
                "rules": {
                    "tool:write_file": {
                        "subject": { "kind": "tool", "tool_id": "write_file" },
                        "behavior": "deny",
                        "scope": "thread",
                        "source": "runtime"
                    }
                }
            },
            "permissions": {
                "default_behavior": "allow",
                "tools": {
                    "write_file": "allow"
                }
            }
        });

        assert_eq!(
            resolve_permission_behavior(&snapshot, "write_file"),
            ToolPermissionBehavior::Deny
        );
    }

    #[test]
    fn permission_confirmation_ticket_carries_message_and_schema() {
        let ticket =
            permission_confirmation_ticket("call_1", "write_file", json!({"path": "a.txt"}));
        assert_eq!(ticket.suspension.action, "tool:PermissionConfirm");
        assert_eq!(ticket.pending.name, "PermissionConfirm");
        assert_eq!(ticket.pending.arguments["tool_name"], "write_file");
        assert!(ticket.suspension.message.contains("write_file"));
        assert!(ticket.suspension.response_schema.is_some());
    }

    #[test]
    fn enforce_permission_ask_without_call_id_blocks() {
        let evaluation = PermissionEvaluation {
            subject: PermissionSubject::tool("write_file"),
            behavior: ToolPermissionBehavior::Ask,
            matched_rule: None,
        };
        let outcome = enforce_permission(
            PermissionMechanismInput {
                tool_id: "write_file",
                tool_args: json!({}),
                call_id: None,
                resume_action: None,
            },
            &evaluation,
        );
        assert!(matches!(
            outcome,
            PermissionMechanismDecision::Action(BeforeToolExecuteAction::Block(_))
        ));
    }

    fn read_only_ctx<'a>(
        config: &'a RunPolicy,
        doc: &'a DocCell,
        tool_name: &'a str,
        call_id: &'a str,
        args: &'a serde_json::Value,
    ) -> tirea_contract::runtime::behavior::ReadOnlyContext<'a> {
        tirea_contract::runtime::behavior::ReadOnlyContext::new(
            Phase::BeforeToolExecute,
            "t1",
            &[],
            config,
            doc,
        )
        .with_tool_info(tool_name, call_id, Some(args))
    }

    #[tokio::test]
    async fn permission_plugin_allow() {
        let config = RunPolicy::new();
        let doc = DocCell::new(json!({
            "permission_policy": {
                "default_behavior": "allow",
                "rules": {}
            }
        }));
        let args = json!({});
        let ctx = read_only_ctx(&config, &doc, "any_tool", "call_1", &args);
        let actions = AgentBehavior::before_tool_execute(&PermissionPlugin, &ctx).await;
        assert!(!has_block(&actions));
        assert!(suspend_action(&actions).is_none());
    }

    #[tokio::test]
    async fn permission_plugin_deny() {
        let config = RunPolicy::new();
        let doc = DocCell::new(json!({
            "permission_policy": {
                "default_behavior": "deny",
                "rules": {}
            }
        }));
        let args = json!({});
        let ctx = read_only_ctx(&config, &doc, "any_tool", "call_1", &args);
        let actions = AgentBehavior::before_tool_execute(&PermissionPlugin, &ctx).await;
        assert!(has_block(&actions));
    }

    #[tokio::test]
    async fn permission_plugin_ask_suspends_with_tool_like_form() {
        let config = RunPolicy::new();
        let doc = DocCell::new(json!({
            "permission_policy": {
                "default_behavior": "ask",
                "rules": {}
            }
        }));
        let args = json!({"path": "a.txt"});
        let ctx = read_only_ctx(&config, &doc, "test_tool", "call_1", &args);
        let actions = AgentBehavior::before_tool_execute(&PermissionPlugin, &ctx).await;
        let ticket = suspend_action(&actions).expect("permission ask should suspend");
        assert_eq!(ticket.pending.name, "PermissionConfirm");
        assert_eq!(ticket.pending.arguments["tool_name"], "test_tool");
        assert!(ticket.suspension.message.contains("test_tool"));
        assert!(ticket.suspension.response_schema.is_some());
    }

    #[tokio::test]
    async fn permission_plugin_resume_bypasses_follow_up_prompt() {
        let config = RunPolicy::new();
        let doc = DocCell::new(json!({
            "permission_policy": {
                "default_behavior": "ask",
                "rules": {}
            }
        }));
        let args = json!({});
        let resume = ToolCallResume {
            decision_id: "decision_fc_call_1".to_string(),
            action: ResumeDecisionAction::Resume,
            result: serde_json::Value::Bool(true),
            reason: None,
            updated_at: 1,
        };
        let ctx =
            read_only_ctx(&config, &doc, "test_tool", "call_1", &args).with_resume_input(resume);
        let actions = AgentBehavior::before_tool_execute(&PermissionPlugin, &ctx).await;
        assert!(!has_block(&actions));
        assert!(suspend_action(&actions).is_none());
    }

    #[test]
    fn tool_policy_plugin_id() {
        assert_eq!(AgentBehavior::id(&ToolPolicyPlugin), "tool_policy");
    }

    #[tokio::test]
    async fn tool_policy_blocks_out_of_scope() {
        let mut config = RunPolicy::new();
        config.set_allowed_tools_if_absent(Some(&["other_tool".to_string()]));
        let doc = DocCell::new(json!({}));
        let args = json!({});
        let ctx = read_only_ctx(&config, &doc, "blocked_tool", "call_1", &args);
        let actions = AgentBehavior::before_tool_execute(&ToolPolicyPlugin, &ctx).await;
        assert!(has_block(&actions));
    }
}
