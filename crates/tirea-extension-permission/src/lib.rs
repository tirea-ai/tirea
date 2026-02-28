//! Permission management extension for Context.
//!
//! Provides methods for managing tool permissions:
//! - `allow_tool(id)` - Allow tool without confirmation
//! - `deny_tool(id)` - Deny tool execution
//! - `ask_tool(id)` - Require confirmation for tool
//! - `get_permission(id)` - Get current permission for tool
//!
//! Also provides `PermissionPlugin` that enforces permissions before tool execution.
//!
//! # Example
//!
//! ```ignore
//! use tirea::prelude::*;
//!
//! // In a tool implementation
//! async fn execute(&self, args: Value, ctx: &ContextAgentState) -> Result<ToolResult, ToolError> {
//!     // Allow follow-up tool after this one
//!     ctx.allow_tool("follow_up_tool");
//!     Ok(ToolResult::success("my_tool", json!({})))
//! }
//! ```

pub mod scope;
pub use scope::*;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::HashMap;
use tirea_contract::io::ResumeDecisionAction;
use tirea_contract::runtime::plugin::agent::{AgentBehavior, ReadOnlyContext};
use tirea_contract::runtime::plugin::phase::effect::PhaseOutput;
use tirea_contract::runtime::plugin::phase::SuspendTicket;
use tirea_contract::runtime::tool_call::ToolCallContext;
use tirea_contract::runtime::{PendingToolCall, ToolCallResumeMode};
use tirea_state::State;

/// Tool permission behavior.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolPermissionBehavior {
    /// Tool is allowed without confirmation.
    Allow,
    /// Tool requires user confirmation before execution.
    #[default]
    Ask,
    /// Tool is denied (will not execute).
    Deny,
}

/// Persisted permission state.
#[derive(Debug, Clone, Default, Serialize, Deserialize, State)]
#[tirea(path = "permissions")]
pub struct PermissionState {
    /// Default behavior for tools not explicitly configured.
    pub default_behavior: ToolPermissionBehavior,
    /// Per-tool permission overrides.
    pub tools: HashMap<String, ToolPermissionBehavior>,
}

/// Frontend tool name for permission confirmation prompts.
pub const PERMISSION_CONFIRM_TOOL_NAME: &str = "PermissionConfirm";

/// Extension trait for permission management on Context.
pub trait PermissionContextExt {
    /// Allow a tool to execute without confirmation.
    fn allow_tool(&self, tool_id: impl Into<String>);

    /// Deny a tool from executing.
    fn deny_tool(&self, tool_id: impl Into<String>);

    /// Require confirmation before tool execution.
    fn ask_tool(&self, tool_id: impl Into<String>);

    /// Get the permission behavior for a tool.
    fn get_permission(&self, tool_id: &str) -> ToolPermissionBehavior;

    /// Set the default permission behavior.
    fn set_default_permission(&self, behavior: ToolPermissionBehavior);

    /// Get the default permission behavior.
    fn get_default_permission(&self) -> ToolPermissionBehavior;
}

impl PermissionContextExt for ToolCallContext<'_> {
    fn allow_tool(&self, tool_id: impl Into<String>) {
        let state = self.state_of::<PermissionState>();
        let _ = state.tools_insert(tool_id.into(), ToolPermissionBehavior::Allow);
    }

    fn deny_tool(&self, tool_id: impl Into<String>) {
        let state = self.state_of::<PermissionState>();
        let _ = state.tools_insert(tool_id.into(), ToolPermissionBehavior::Deny);
    }

    fn ask_tool(&self, tool_id: impl Into<String>) {
        let state = self.state_of::<PermissionState>();
        let _ = state.tools_insert(tool_id.into(), ToolPermissionBehavior::Ask);
    }

    fn get_permission(&self, tool_id: &str) -> ToolPermissionBehavior {
        let state = self.state_of::<PermissionState>();
        if let Ok(tools) = state.tools() {
            if let Some(permission) = tools.get(tool_id) {
                return *permission;
            }
        }
        state.default_behavior().ok().unwrap_or_default()
    }

    fn set_default_permission(&self, behavior: ToolPermissionBehavior) {
        let state = self.state_of::<PermissionState>();
        let _ = state.set_default_behavior(behavior);
    }

    fn get_default_permission(&self) -> ToolPermissionBehavior {
        let state = self.state_of::<PermissionState>();
        state.default_behavior().ok().unwrap_or_default()
    }
}

/// Permission strategy plugin.
///
/// This plugin checks permissions in `before_tool_execute`.
/// - `Allow`: no-op
/// - `Deny`: block tool
/// - `Ask`: suspend the tool call and emit a confirmation ticket
pub struct PermissionPlugin;

#[async_trait]
impl AgentBehavior for PermissionPlugin {
    fn id(&self) -> &str {
        "permission"
    }

    async fn before_tool_execute(&self, ctx: &ReadOnlyContext<'_>) -> PhaseOutput {
        let Some(tool_id) = ctx.tool_name() else {
            return PhaseOutput::default();
        };

        let call_id = ctx.tool_call_id().unwrap_or_default().to_string();
        if !call_id.is_empty() {
            let has_resume_grant = ctx
                .resume_input()
                .is_some_and(|resume| matches!(resume.action, ResumeDecisionAction::Resume));
            if has_resume_grant {
                return PhaseOutput::default();
            }
        }

        // Read permission fields individually to tolerate partial corruption.
        // If a specific tool entry has an invalid value, fall back to the
        // default_behavior field. If default_behavior is also invalid, fall
        // back to Ask (the Default).
        let snapshot = ctx.snapshot();
        let perms = snapshot
            .get("permissions")
            .unwrap_or(&serde_json::Value::Null);

        let tool_permission = perms
            .get("tools")
            .and_then(|tools| tools.get(tool_id))
            .and_then(|v| serde_json::from_value::<ToolPermissionBehavior>(v.clone()).ok());

        let permission = tool_permission.unwrap_or_else(|| {
            perms
                .get("default_behavior")
                .and_then(|v| serde_json::from_value::<ToolPermissionBehavior>(v.clone()).ok())
                .unwrap_or_default()
        });

        match permission {
            ToolPermissionBehavior::Allow => PhaseOutput::default(),
            ToolPermissionBehavior::Deny => {
                PhaseOutput::new().block_tool(format!("Tool '{}' is denied", tool_id))
            }
            ToolPermissionBehavior::Ask => {
                if call_id.is_empty() {
                    return PhaseOutput::new()
                        .block_tool("Permission check requires non-empty tool call id");
                }
                let tool_args = ctx.tool_args().cloned().unwrap_or_default();
                let arguments = json!({
                    "tool_name": tool_id,
                    "tool_args": tool_args.clone(),
                });
                let pending_call_id = format!("fc_{call_id}");
                let suspension =
                    tirea_contract::Suspension::new(&pending_call_id, "tool:PermissionConfirm")
                        .with_parameters(arguments.clone());
                PhaseOutput::new().suspend_tool(SuspendTicket::new(
                    suspension,
                    PendingToolCall::new(pending_call_id, PERMISSION_CONFIRM_TOOL_NAME, arguments),
                    ToolCallResumeMode::ReplayToolCall,
                ))
            }
        }
    }
}

/// Tool scope policy plugin.
///
/// Enforces allow/deny list filtering for tools via `RunConfig` scope keys.
/// Should be installed before `PermissionPlugin` so that out-of-scope tools
/// are blocked before per-tool permission checks run.
pub struct ToolPolicyPlugin;

#[async_trait]
impl AgentBehavior for ToolPolicyPlugin {
    fn id(&self) -> &str {
        "tool_policy"
    }

    async fn before_inference(&self, ctx: &ReadOnlyContext<'_>) -> PhaseOutput {
        let run_config = ctx.run_config();
        let allowed = scope::parse_scope_filter(run_config.value(SCOPE_ALLOWED_TOOLS_KEY));
        let excluded = scope::parse_scope_filter(run_config.value(SCOPE_EXCLUDED_TOOLS_KEY));

        let mut output = PhaseOutput::new();
        if let Some(ref allowed) = allowed {
            output = output.include_only_tools(allowed.clone());
        }
        if let Some(ref excluded) = excluded {
            for id in excluded {
                output = output.exclude_tool(id);
            }
        }
        output
    }

    async fn before_tool_execute(&self, ctx: &ReadOnlyContext<'_>) -> PhaseOutput {
        let Some(tool_id) = ctx.tool_name() else {
            return PhaseOutput::default();
        };

        let run_config = ctx.run_config();
        if !scope::is_scope_allowed(
            Some(run_config),
            tool_id,
            SCOPE_ALLOWED_TOOLS_KEY,
            SCOPE_EXCLUDED_TOOLS_KEY,
        ) {
            PhaseOutput::new().block_tool(format!(
                "Tool '{}' is not allowed by current policy",
                tool_id
            ))
        } else {
            PhaseOutput::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tirea_contract::io::ResumeDecisionAction;
    use tirea_contract::runtime::plugin::phase::effect::PhaseEffect;
    use tirea_contract::runtime::plugin::phase::Phase;
    use tirea_contract::runtime::tool_call::ToolCallResume;
    use tirea_contract::testing::TestFixture;
    use tirea_contract::RunConfig;
    use tirea_state::DocCell;

    fn has_block(output: &PhaseOutput) -> bool {
        output
            .effects
            .iter()
            .any(|e| matches!(e, PhaseEffect::BlockTool(_)))
    }

    fn has_suspend(output: &PhaseOutput) -> bool {
        output
            .effects
            .iter()
            .any(|e| matches!(e, PhaseEffect::SuspendTool(_)))
    }

    fn extract_suspend_ticket(output: &PhaseOutput) -> Option<&SuspendTicket> {
        output.effects.iter().find_map(|e| match e {
            PhaseEffect::SuspendTool(ticket) => Some(ticket),
            _ => None,
        })
    }

    #[test]
    fn test_permission_state_default() {
        let state = PermissionState::default();
        assert_eq!(state.default_behavior, ToolPermissionBehavior::Ask);
        assert!(state.tools.is_empty());
    }

    #[test]
    fn test_permission_state_serialization() {
        let mut state = PermissionState::default();
        state
            .tools
            .insert("read".to_string(), ToolPermissionBehavior::Allow);

        let json = serde_json::to_string(&state).unwrap();
        let parsed: PermissionState = serde_json::from_str(&json).unwrap();

        assert_eq!(
            parsed.tools.get("read"),
            Some(&ToolPermissionBehavior::Allow)
        );
    }

    #[test]
    fn test_get_permission_prefers_tool_override() {
        let fix = TestFixture::new_with_state(json!({
            "permissions": {
                "default_behavior": "deny",
                "tools": {
                    "recover_agent_run": "allow"
                }
            }
        }));
        let ctx = fix.ctx();
        assert_eq!(
            ctx.get_permission("recover_agent_run"),
            ToolPermissionBehavior::Allow
        );
    }

    #[test]
    fn test_get_permission_falls_back_to_default() {
        let fix = TestFixture::new_with_state(json!({
            "permissions": {
                "default_behavior": "deny",
                "tools": {}
            }
        }));
        let ctx = fix.ctx();
        assert_eq!(
            ctx.get_permission("unknown_tool"),
            ToolPermissionBehavior::Deny
        );
    }

    #[test]
    fn test_get_permission_missing_state_falls_back_to_ask() {
        let fix = TestFixture::new();
        let ctx = fix.ctx();
        assert_eq!(
            ctx.get_permission("recover_agent_run"),
            ToolPermissionBehavior::Ask
        );
    }

    #[test]
    fn test_allow_tool() {
        let fix = TestFixture::new_with_state(json!({
            "permissions": {
                "default_behavior": "ask",
                "tools": {}
            }
        }));
        let ctx = fix.ctx();

        ctx.allow_tool("read_file");
        assert!(fix.has_changes());
    }

    #[test]
    fn test_deny_tool() {
        let fix = TestFixture::new_with_state(json!({
            "permissions": {
                "default_behavior": "ask",
                "tools": {}
            }
        }));
        let ctx = fix.ctx();

        ctx.deny_tool("delete_file");
        assert!(fix.has_changes());
    }

    #[test]
    fn test_ask_tool() {
        let fix = TestFixture::new_with_state(json!({
            "permissions": {
                "default_behavior": "allow",
                "tools": {}
            }
        }));
        let ctx = fix.ctx();

        ctx.ask_tool("write_file");
        assert!(fix.has_changes());
    }

    #[test]
    fn test_get_permission_default() {
        let fix = TestFixture::new_with_state(json!({
            "permissions": {
                "default_behavior": "deny",
                "tools": {}
            }
        }));
        let ctx = fix.ctx();

        assert_eq!(
            ctx.get_permission("unknown_tool"),
            ToolPermissionBehavior::Deny
        );
    }

    #[test]
    fn test_get_permission_override() {
        let fix = TestFixture::new_with_state(json!({
            "permissions": {
                "default_behavior": "deny",
                "tools": { "special_tool": "allow" }
            }
        }));
        let ctx = fix.ctx();

        assert_eq!(
            ctx.get_permission("special_tool"),
            ToolPermissionBehavior::Allow
        );
        assert_eq!(
            ctx.get_permission("other_tool"),
            ToolPermissionBehavior::Deny
        );
    }

    #[test]
    fn test_set_default_permission() {
        let fix = TestFixture::new_with_state(json!({
            "permissions": {
                "default_behavior": "ask",
                "tools": {}
            }
        }));
        let ctx = fix.ctx();

        ctx.set_default_permission(ToolPermissionBehavior::Allow);
        assert!(fix.has_changes());
    }

    #[test]
    fn test_permission_plugin_id() {
        let plugin = PermissionPlugin;
        assert_eq!(AgentBehavior::id(&plugin), "permission");
    }

    #[tokio::test]
    async fn test_permission_plugin_allow() {
        let config = RunConfig::new();
        let doc =
            DocCell::new(json!({ "permissions": { "default_behavior": "allow", "tools": {} } }));
        let args = json!({});
        let ctx = ReadOnlyContext::new(Phase::BeforeToolExecute, "t1", &[], &config, &doc)
            .with_tool_info("any_tool", "call_1", Some(&args));

        let output = AgentBehavior::before_tool_execute(&PermissionPlugin, &ctx).await;
        assert!(!has_block(&output));
        assert!(!has_suspend(&output));
    }

    #[tokio::test]
    async fn test_permission_plugin_deny() {
        let config = RunConfig::new();
        let doc =
            DocCell::new(json!({ "permissions": { "default_behavior": "deny", "tools": {} } }));
        let args = json!({});
        let ctx = ReadOnlyContext::new(Phase::BeforeToolExecute, "t1", &[], &config, &doc)
            .with_tool_info("any_tool", "call_1", Some(&args));

        let output = AgentBehavior::before_tool_execute(&PermissionPlugin, &ctx).await;
        assert!(has_block(&output));
    }

    #[tokio::test]
    async fn test_permission_plugin_ask() {
        let config = RunConfig::new();
        let doc =
            DocCell::new(json!({ "permissions": { "default_behavior": "ask", "tools": {} } }));
        let args = json!({"path": "a.txt"});
        let ctx = ReadOnlyContext::new(Phase::BeforeToolExecute, "t1", &[], &config, &doc)
            .with_tool_info("test_tool", "call_1", Some(&args));

        let output = AgentBehavior::before_tool_execute(&PermissionPlugin, &ctx).await;
        assert!(has_suspend(&output));

        let ticket = extract_suspend_ticket(&output).expect("suspend ticket should exist");
        assert_eq!(
            ticket.suspension.action,
            format!("tool:{}", PERMISSION_CONFIRM_TOOL_NAME)
        );
        assert_eq!(ticket.pending.id, "fc_call_1");
        assert_eq!(ticket.pending.name, PERMISSION_CONFIRM_TOOL_NAME);
        assert_eq!(ticket.pending.arguments["tool_name"], "test_tool");
        assert_eq!(ticket.pending.arguments["tool_args"]["path"], "a.txt");
        assert_eq!(ticket.resume_mode, ToolCallResumeMode::ReplayToolCall);
    }

    #[tokio::test]
    async fn test_permission_plugin_ask_with_empty_call_id_blocks() {
        let config = RunConfig::new();
        let doc =
            DocCell::new(json!({ "permissions": { "default_behavior": "ask", "tools": {} } }));
        let args = json!({"path": "a.txt"});
        let ctx = ReadOnlyContext::new(Phase::BeforeToolExecute, "t1", &[], &config, &doc)
            .with_tool_info("test_tool", "", Some(&args));

        let output = AgentBehavior::before_tool_execute(&PermissionPlugin, &ctx).await;
        assert!(has_block(&output));
        assert!(!has_suspend(&output));
    }

    #[test]
    fn test_get_default_permission() {
        let fix = TestFixture::new_with_state(json!({
            "permissions": {
                "default_behavior": "allow",
                "tools": {}
            }
        }));
        let ctx = fix.ctx();

        let default = ctx.get_default_permission();
        assert_eq!(default, ToolPermissionBehavior::Allow);
    }

    #[test]
    fn test_get_default_permission_deny() {
        let fix = TestFixture::new_with_state(json!({
            "permissions": {
                "default_behavior": "deny",
                "tools": {}
            }
        }));
        let ctx = fix.ctx();

        let default = ctx.get_default_permission();
        assert_eq!(default, ToolPermissionBehavior::Deny);
    }

    #[test]
    fn test_get_default_permission_fallback() {
        let fix = TestFixture::new();
        let ctx = fix.ctx();

        let default = ctx.get_default_permission();
        assert_eq!(default, ToolPermissionBehavior::Ask);
    }

    #[tokio::test]
    async fn test_permission_plugin_tool_specific_allow() {
        let config = RunConfig::new();
        let doc = DocCell::new(
            json!({ "permissions": { "default_behavior": "deny", "tools": { "allowed_tool": "allow" } } }),
        );
        let args = json!({});
        let ctx = ReadOnlyContext::new(Phase::BeforeToolExecute, "t1", &[], &config, &doc)
            .with_tool_info("allowed_tool", "call_1", Some(&args));

        let output = AgentBehavior::before_tool_execute(&PermissionPlugin, &ctx).await;
        assert!(!has_block(&output));
    }

    #[tokio::test]
    async fn test_permission_plugin_tool_specific_deny() {
        let config = RunConfig::new();
        let doc = DocCell::new(
            json!({ "permissions": { "default_behavior": "allow", "tools": { "denied_tool": "deny" } } }),
        );
        let args = json!({});
        let ctx = ReadOnlyContext::new(Phase::BeforeToolExecute, "t1", &[], &config, &doc)
            .with_tool_info("denied_tool", "call_1", Some(&args));

        let output = AgentBehavior::before_tool_execute(&PermissionPlugin, &ctx).await;
        assert!(has_block(&output));
    }

    #[tokio::test]
    async fn test_permission_plugin_tool_specific_ask() {
        let config = RunConfig::new();
        let doc = DocCell::new(
            json!({ "permissions": { "default_behavior": "allow", "tools": { "ask_tool": "ask" } } }),
        );
        let args = json!({});
        let ctx = ReadOnlyContext::new(Phase::BeforeToolExecute, "t1", &[], &config, &doc)
            .with_tool_info("ask_tool", "call_1", Some(&args));

        let output = AgentBehavior::before_tool_execute(&PermissionPlugin, &ctx).await;
        assert!(has_suspend(&output));
    }

    #[tokio::test]
    async fn test_permission_plugin_invalid_tool_behavior() {
        let config = RunConfig::new();
        let doc = DocCell::new(
            json!({ "permissions": { "default_behavior": "allow", "tools": { "invalid_tool": "invalid_behavior" } } }),
        );
        let args = json!({});
        let ctx = ReadOnlyContext::new(Phase::BeforeToolExecute, "t1", &[], &config, &doc)
            .with_tool_info("invalid_tool", "call_1", Some(&args));

        let output = AgentBehavior::before_tool_execute(&PermissionPlugin, &ctx).await;
        // Should fall back to default "allow" behavior
        assert!(!has_block(&output));
        assert!(!has_suspend(&output));
    }

    #[tokio::test]
    async fn test_permission_plugin_invalid_default_behavior() {
        let config = RunConfig::new();
        let doc = DocCell::new(
            json!({ "permissions": { "default_behavior": "invalid_default", "tools": {} } }),
        );
        let args = json!({});
        let ctx = ReadOnlyContext::new(Phase::BeforeToolExecute, "t1", &[], &config, &doc)
            .with_tool_info("any_tool", "call_1", Some(&args));

        let output = AgentBehavior::before_tool_execute(&PermissionPlugin, &ctx).await;
        // Should fall back to Ask behavior
        assert!(has_suspend(&output));
    }

    #[tokio::test]
    async fn test_permission_plugin_no_state() {
        // Thread with no permission state at all — should default to Ask
        let config = RunConfig::new();
        let doc = DocCell::new(json!({}));
        let args = json!({});
        let ctx = ReadOnlyContext::new(Phase::BeforeToolExecute, "t1", &[], &config, &doc)
            .with_tool_info("any_tool", "call_1", Some(&args));

        let output = AgentBehavior::before_tool_execute(&PermissionPlugin, &ctx).await;
        assert!(has_suspend(&output));
    }

    // ========================================================================
    // Corrupted / unexpected state shape fallback tests
    // ========================================================================

    #[tokio::test]
    async fn test_permission_plugin_tools_is_string_not_object() {
        let config = RunConfig::new();
        let doc = DocCell::new(
            json!({ "permissions": { "default_behavior": "allow", "tools": "corrupted" } }),
        );
        let args = json!({});
        let ctx = ReadOnlyContext::new(Phase::BeforeToolExecute, "t1", &[], &config, &doc)
            .with_tool_info("any_tool", "call_1", Some(&args));

        let output = AgentBehavior::before_tool_execute(&PermissionPlugin, &ctx).await;
        // Falls back to default "allow" behavior
        assert!(!has_block(&output));
        assert!(!has_suspend(&output));
    }

    #[tokio::test]
    async fn test_permission_plugin_default_behavior_invalid_string() {
        let config = RunConfig::new();
        let doc = DocCell::new(
            json!({ "permissions": { "default_behavior": "invalid_value", "tools": {} } }),
        );
        let args = json!({});
        let ctx = ReadOnlyContext::new(Phase::BeforeToolExecute, "t1", &[], &config, &doc)
            .with_tool_info("any_tool", "call_1", Some(&args));

        let output = AgentBehavior::before_tool_execute(&PermissionPlugin, &ctx).await;
        // Falls back to Ask
        assert!(has_suspend(&output));
    }

    #[tokio::test]
    async fn test_permission_plugin_default_behavior_is_number() {
        let config = RunConfig::new();
        let doc = DocCell::new(json!({ "permissions": { "default_behavior": 42, "tools": {} } }));
        let args = json!({});
        let ctx = ReadOnlyContext::new(Phase::BeforeToolExecute, "t1", &[], &config, &doc)
            .with_tool_info("any_tool", "call_1", Some(&args));

        let output = AgentBehavior::before_tool_execute(&PermissionPlugin, &ctx).await;
        // Falls back to Ask
        assert!(has_suspend(&output));
    }

    #[tokio::test]
    async fn test_permission_plugin_tool_value_is_number() {
        let config = RunConfig::new();
        let doc = DocCell::new(
            json!({ "permissions": { "default_behavior": "allow", "tools": { "my_tool": 123 } } }),
        );
        let args = json!({});
        let ctx = ReadOnlyContext::new(Phase::BeforeToolExecute, "t1", &[], &config, &doc)
            .with_tool_info("my_tool", "call_1", Some(&args));

        let output = AgentBehavior::before_tool_execute(&PermissionPlugin, &ctx).await;
        // Falls back to default "allow"
        assert!(!has_block(&output));
        assert!(!has_suspend(&output));
    }

    #[tokio::test]
    async fn test_permission_plugin_permissions_is_array() {
        let config = RunConfig::new();
        let doc = DocCell::new(json!({ "permissions": [1, 2, 3] }));
        let args = json!({});
        let ctx = ReadOnlyContext::new(Phase::BeforeToolExecute, "t1", &[], &config, &doc)
            .with_tool_info("any_tool", "call_1", Some(&args));

        let output = AgentBehavior::before_tool_execute(&PermissionPlugin, &ctx).await;
        // Falls back to Ask
        assert!(has_suspend(&output));
    }

    // ========================================================================
    // ToolPolicyPlugin tests
    // ========================================================================

    #[test]
    fn test_tool_policy_plugin_id() {
        assert_eq!(AgentBehavior::id(&ToolPolicyPlugin), "tool_policy");
    }

    #[tokio::test]
    async fn test_tool_policy_blocks_out_of_scope() {
        let mut config = RunConfig::new();
        config
            .set(scope::SCOPE_ALLOWED_TOOLS_KEY, vec!["other_tool"])
            .unwrap();
        let doc = DocCell::new(json!({}));
        let args = json!({});
        let ctx = ReadOnlyContext::new(Phase::BeforeToolExecute, "t1", &[], &config, &doc)
            .with_tool_info("blocked_tool", "call_1", Some(&args));

        let output = AgentBehavior::before_tool_execute(&ToolPolicyPlugin, &ctx).await;
        assert!(has_block(&output), "out-of-scope tool should be blocked");
    }

    #[tokio::test]
    async fn test_tool_policy_allows_in_scope() {
        let mut config = RunConfig::new();
        config
            .set(scope::SCOPE_ALLOWED_TOOLS_KEY, vec!["my_tool"])
            .unwrap();
        let doc = DocCell::new(json!({}));
        let args = json!({});
        let ctx = ReadOnlyContext::new(Phase::BeforeToolExecute, "t1", &[], &config, &doc)
            .with_tool_info("my_tool", "call_1", Some(&args));

        let output = AgentBehavior::before_tool_execute(&ToolPolicyPlugin, &ctx).await;
        assert!(!has_block(&output));
    }

    #[tokio::test]
    async fn test_tool_policy_no_filters_allows_all() {
        let config = RunConfig::new();
        let doc = DocCell::new(json!({}));
        let args = json!({});
        let ctx = ReadOnlyContext::new(Phase::BeforeToolExecute, "t1", &[], &config, &doc)
            .with_tool_info("any_tool", "call_1", Some(&args));

        let output = AgentBehavior::before_tool_execute(&ToolPolicyPlugin, &ctx).await;
        assert!(!has_block(&output));
    }

    #[tokio::test]
    async fn test_tool_policy_excluded_tool_is_blocked() {
        let mut config = RunConfig::new();
        config
            .set(scope::SCOPE_EXCLUDED_TOOLS_KEY, vec!["excluded_tool"])
            .unwrap();
        let doc = DocCell::new(json!({}));
        let args = json!({});
        let ctx = ReadOnlyContext::new(Phase::BeforeToolExecute, "t1", &[], &config, &doc)
            .with_tool_info("excluded_tool", "call_1", Some(&args));

        let output = AgentBehavior::before_tool_execute(&ToolPolicyPlugin, &ctx).await;
        assert!(has_block(&output), "excluded tool should be blocked");
    }

    #[tokio::test]
    async fn test_permission_resume_input_bypasses_ask() {
        let config = RunConfig::new();
        let doc = DocCell::new(json!({
            "permissions": {
                "default_behavior": "ask",
                "tools": {}
            }
        }));
        let args = json!({});
        let resume = ToolCallResume {
            decision_id: "fc_call_1".to_string(),
            action: ResumeDecisionAction::Resume,
            result: serde_json::Value::Bool(true),
            reason: None,
            updated_at: 1,
        };
        let ctx = ReadOnlyContext::new(Phase::BeforeToolExecute, "t1", &[], &config, &doc)
            .with_tool_info("test_tool", "call_1", Some(&args))
            .with_resume_input(resume);

        let output = AgentBehavior::before_tool_execute(&PermissionPlugin, &ctx).await;
        assert!(
            !has_block(&output),
            "resume-approved call should be allowed"
        );
        assert!(
            !has_suspend(&output),
            "resume-approved call should not suspend again"
        );
    }
}
