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

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::HashMap;
use tirea_contract::event::suspension::{
    FrontendToolInvocation, InvocationOrigin, ResponseRouting,
};
use tirea_contract::plugin::phase::{
    BeforeToolExecuteContext, PluginPhaseContext, SuspendTicket, ToolGateDecision,
};
use tirea_contract::plugin::AgentPlugin;
use tirea_contract::runtime::control::ResumeDecisionAction;
use tirea_contract::tool::context::ToolCallContext;
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
impl AgentPlugin for PermissionPlugin {
    fn id(&self) -> &str {
        "permission"
    }

    async fn before_tool_execute(&self, step: &mut BeforeToolExecuteContext<'_, '_>) {
        if !matches!(step.decision(), ToolGateDecision::Proceed) {
            return;
        }

        let Some(tool_id) = step.tool_name() else {
            return;
        };

        // Resumed calls carry decision payload on per-call lifecycle state.
        let call_id = step.tool_call_id().unwrap_or_default().to_string();
        if !call_id.is_empty() {
            let has_resume_grant = step
                .resume_input()
                .is_some_and(|resume| matches!(resume.action, ResumeDecisionAction::Resume));
            if has_resume_grant {
                return;
            }
        }

        let permission = {
            let state = step.state_of::<PermissionState>();
            if let Ok(tools) = state.tools() {
                if let Some(permission) = tools.get(tool_id) {
                    *permission
                } else {
                    state.default_behavior().ok().unwrap_or_default()
                }
            } else {
                state.default_behavior().ok().unwrap_or_default()
            }
        };

        match permission {
            ToolPermissionBehavior::Allow => {
                // Allowed - do nothing
            }
            ToolPermissionBehavior::Deny => {
                step.block(format!("Tool '{}' is denied", tool_id));
            }
            ToolPermissionBehavior::Ask => {
                if call_id.is_empty() {
                    step.block("Permission check requires non-empty tool call id");
                    return;
                }
                let tool_args = step.tool_args().cloned().unwrap_or_default();
                let arguments = json!({
                    "tool_name": tool_id,
                    "tool_args": tool_args.clone(),
                });
                let invocation = FrontendToolInvocation::new(
                    format!("fc_{call_id}"),
                    PERMISSION_CONFIRM_TOOL_NAME,
                    arguments,
                    InvocationOrigin::ToolCallIntercepted {
                        backend_call_id: call_id,
                        backend_tool_name: tool_id.to_string(),
                        backend_arguments: tool_args,
                    },
                    ResponseRouting::ReplayOriginalTool,
                );
                step.suspend(SuspendTicket::from_invocation(invocation));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tirea_contract::plugin::phase::{BeforeToolExecuteContext, ToolContext};
    use tirea_contract::testing::TestFixture;
    use tirea_contract::thread::ToolCall;

    fn apply_interaction_intents(_step: &mut tirea_contract::plugin::phase::StepContext<'_>) {
        // No-op: permission plugin now writes suspend tickets directly.
    }

    async fn run_before_tool_execute(
        plugin: &PermissionPlugin,
        step: &mut tirea_contract::plugin::phase::StepContext<'_>,
    ) {
        let mut ctx = BeforeToolExecuteContext::new(step);
        plugin.before_tool_execute(&mut ctx).await;
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
        // Note: We can't verify get_permission() returns Allow because
        // Context collects ops that need to be applied to see the result.
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
        // Note: We can't verify get_permission() returns Deny because
        // Context collects ops that need to be applied to see the result.
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
        // Note: We can't verify get_permission() returns Ask because
        // Context collects ops that need to be applied to see the result.
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

        // Tool not in tools map should return default
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
        // Note: We can't verify get_default_permission() returns Allow because
        // Context collects ops that need to be applied to see the result.
        assert!(fix.has_changes());
    }

    #[test]
    fn test_permission_plugin_id() {
        let plugin = PermissionPlugin;
        assert_eq!(plugin.id(), "permission");
    }

    #[tokio::test]
    async fn test_permission_plugin_allow() {
        let fixture = TestFixture::new_with_state(
            json!({ "permissions": { "default_behavior": "allow", "tools": {} } }),
        );
        let mut step = fixture.step(vec![]);

        let call = ToolCall::new("call_1", "any_tool", json!({}));
        step.tool = Some(ToolContext::new(&call));

        let plugin = PermissionPlugin;
        run_before_tool_execute(&plugin, &mut step).await;
        apply_interaction_intents(&mut step);

        assert!(!step.tool_blocked());
        assert!(!step.tool_pending());
    }

    #[tokio::test]
    async fn test_permission_plugin_deny() {
        let fixture = TestFixture::new_with_state(
            json!({ "permissions": { "default_behavior": "deny", "tools": {} } }),
        );
        let mut step = fixture.step(vec![]);

        let call = ToolCall::new("call_1", "any_tool", json!({}));
        step.tool = Some(ToolContext::new(&call));

        let plugin = PermissionPlugin;
        run_before_tool_execute(&plugin, &mut step).await;
        apply_interaction_intents(&mut step);

        assert!(step.tool_blocked());
    }

    #[tokio::test]
    async fn test_permission_plugin_ask() {
        use tirea_contract::event::suspension::{InvocationOrigin, ResponseRouting};

        let fixture = TestFixture::new_with_state(
            json!({ "permissions": { "default_behavior": "ask", "tools": {} } }),
        );
        let mut step = fixture.step(vec![]);

        let call = ToolCall::new("call_1", "test_tool", json!({"path": "a.txt"}));
        step.tool = Some(ToolContext::new(&call));

        let plugin = PermissionPlugin;
        run_before_tool_execute(&plugin, &mut step).await;
        apply_interaction_intents(&mut step);

        assert!(step.tool_pending());

        // Should expose suspension interaction payload
        let interaction = step
            .tool
            .as_ref()
            .and_then(|t| t.suspend_ticket.as_ref())
            .map(|ticket| &ticket.suspension)
            .expect("suspended interaction should exist");
        assert_eq!(
            interaction.action,
            format!("tool:{}", PERMISSION_CONFIRM_TOOL_NAME)
        );

        // Should have first-class FrontendToolInvocation
        let inv = step
            .tool
            .as_ref()
            .and_then(|t| t.suspend_ticket.as_ref())
            .map(|ticket| &ticket.invocation)
            .expect("pending frontend invocation should exist");
        assert_eq!(inv.tool_name, PERMISSION_CONFIRM_TOOL_NAME);
        assert_eq!(inv.arguments["tool_name"], "test_tool");
        assert_eq!(inv.arguments["tool_args"]["path"], "a.txt");

        // Origin should be ToolCallIntercepted with original backend tool info
        match &inv.origin {
            InvocationOrigin::ToolCallIntercepted {
                backend_call_id,
                backend_tool_name,
                backend_arguments,
            } => {
                assert_eq!(backend_call_id, "call_1");
                assert_eq!(backend_tool_name, "test_tool");
                assert_eq!(backend_arguments["path"], "a.txt");
            }
            _ => panic!("Expected ToolCallIntercepted origin"),
        }

        // Routing should be ReplayOriginalTool without embedded state patches.
        match &inv.routing {
            ResponseRouting::ReplayOriginalTool => {}
            _ => panic!("Expected ReplayOriginalTool routing"),
        }

        let routing_json = serde_json::to_value(&inv.routing).expect("routing should serialize");
        assert!(
            routing_json.get("state_patches").is_none(),
            "routing must not serialize legacy state_patches field: {routing_json:?}"
        );
    }

    #[tokio::test]
    async fn test_permission_plugin_skips_when_tool_already_pending() {
        use tirea_contract::event::suspension::{
            FrontendToolInvocation, InvocationOrigin, ResponseRouting,
        };
        use tirea_contract::plugin::phase::SuspendTicket;

        let fixture = TestFixture::new_with_state(
            json!({ "permissions": { "default_behavior": "ask", "tools": {} } }),
        );
        let mut step = fixture.step(vec![]);
        let call = ToolCall::new("call_1", "copyToClipboard", json!({"text": "hello"}));
        step.tool = Some(ToolContext::new(&call));

        let invocation = FrontendToolInvocation::new(
            "call_1",
            "copyToClipboard",
            json!({"text": "hello"}),
            InvocationOrigin::PluginInitiated {
                plugin_id: "agui_frontend_tools".to_string(),
            },
            ResponseRouting::UseAsToolResult,
        );
        step.suspend(SuspendTicket::from_invocation(invocation));

        let plugin = PermissionPlugin;
        run_before_tool_execute(&plugin, &mut step).await;
        apply_interaction_intents(&mut step);

        assert!(step.tool_pending());
        assert!(!step.tool_blocked());
        let inv = step
            .tool
            .as_ref()
            .and_then(|t| t.suspend_ticket.as_ref())
            .map(|ticket| &ticket.invocation)
            .expect("pending frontend invocation should exist");
        assert_eq!(inv.tool_name, "copyToClipboard");
    }

    #[tokio::test]
    async fn test_permission_plugin_ask_with_empty_call_id_blocks() {
        let fixture = TestFixture::new_with_state(
            json!({ "permissions": { "default_behavior": "ask", "tools": {} } }),
        );
        let mut step = fixture.step(vec![]);

        let call = ToolCall::new("", "test_tool", json!({"path": "a.txt"}));
        step.tool = Some(ToolContext::new(&call));

        let plugin = PermissionPlugin;
        run_before_tool_execute(&plugin, &mut step).await;
        apply_interaction_intents(&mut step);

        assert!(step.tool_blocked());
        assert!(!step.tool_pending());
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

        // Should read default_behavior from state
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
        // When state is missing, should return default (Ask)
        let fix = TestFixture::new();
        let ctx = fix.ctx();

        let default = ctx.get_default_permission();
        assert_eq!(default, ToolPermissionBehavior::Ask);
    }

    #[tokio::test]
    async fn test_permission_plugin_tool_specific_allow() {
        let fixture = TestFixture::new_with_state(
            json!({ "permissions": { "default_behavior": "deny", "tools": { "allowed_tool": "allow" } } }),
        );
        let mut step = fixture.step(vec![]);

        let call = ToolCall::new("call_1", "allowed_tool", json!({}));
        step.tool = Some(ToolContext::new(&call));

        let plugin = PermissionPlugin;
        run_before_tool_execute(&plugin, &mut step).await;
        apply_interaction_intents(&mut step);

        assert!(!step.tool_blocked());
    }

    #[tokio::test]
    async fn test_permission_plugin_tool_specific_deny() {
        let fixture = TestFixture::new_with_state(
            json!({ "permissions": { "default_behavior": "allow", "tools": { "denied_tool": "deny" } } }),
        );
        let mut step = fixture.step(vec![]);

        let call = ToolCall::new("call_1", "denied_tool", json!({}));
        step.tool = Some(ToolContext::new(&call));

        let plugin = PermissionPlugin;
        run_before_tool_execute(&plugin, &mut step).await;
        apply_interaction_intents(&mut step);

        assert!(step.tool_blocked());
    }

    #[tokio::test]
    async fn test_permission_plugin_tool_specific_ask() {
        let fixture = TestFixture::new_with_state(
            json!({ "permissions": { "default_behavior": "allow", "tools": { "ask_tool": "ask" } } }),
        );
        let mut step = fixture.step(vec![]);

        let call = ToolCall::new("call_1", "ask_tool", json!({}));
        step.tool = Some(ToolContext::new(&call));

        let plugin = PermissionPlugin;
        run_before_tool_execute(&plugin, &mut step).await;
        apply_interaction_intents(&mut step);

        assert!(step.tool_pending());
    }

    #[tokio::test]
    async fn test_permission_plugin_invalid_tool_behavior() {
        let fixture = TestFixture::new_with_state(
            json!({ "permissions": { "default_behavior": "allow", "tools": { "invalid_tool": "invalid_behavior" } } }),
        );
        let mut step = fixture.step(vec![]);

        let call = ToolCall::new("call_1", "invalid_tool", json!({}));
        step.tool = Some(ToolContext::new(&call));

        let plugin = PermissionPlugin;
        run_before_tool_execute(&plugin, &mut step).await;
        apply_interaction_intents(&mut step);

        // Should fall back to default "allow" behavior
        assert!(!step.tool_blocked());
        assert!(!step.tool_pending());
    }

    #[tokio::test]
    async fn test_permission_plugin_invalid_default_behavior() {
        let fixture = TestFixture::new_with_state(
            json!({ "permissions": { "default_behavior": "invalid_default", "tools": {} } }),
        );
        let mut step = fixture.step(vec![]);

        let call = ToolCall::new("call_1", "any_tool", json!({}));
        step.tool = Some(ToolContext::new(&call));

        let plugin = PermissionPlugin;
        run_before_tool_execute(&plugin, &mut step).await;
        apply_interaction_intents(&mut step);

        // Should fall back to Ask behavior
        assert!(step.tool_pending());
    }

    #[tokio::test]
    async fn test_permission_plugin_no_state() {
        // Thread with no permission state at all — should default to Ask
        let fixture = TestFixture::new();
        let mut step = fixture.step(vec![]);

        let call = ToolCall::new("call_1", "any_tool", json!({}));
        step.tool = Some(ToolContext::new(&call));

        let plugin = PermissionPlugin;
        run_before_tool_execute(&plugin, &mut step).await;
        apply_interaction_intents(&mut step);

        assert!(step.tool_pending());
    }

    // ========================================================================
    // Corrupted / unexpected state shape fallback tests
    // ========================================================================

    #[tokio::test]
    async fn test_permission_plugin_tools_is_string_not_object() {
        // "tools" is a string instead of an object — should not panic,
        // falls back to default_behavior.
        let fixture = TestFixture::new_with_state(
            json!({ "permissions": { "default_behavior": "allow", "tools": "corrupted" } }),
        );
        let mut step = fixture.step(vec![]);

        let call = ToolCall::new("call_1", "any_tool", json!({}));
        step.tool = Some(ToolContext::new(&call));

        let plugin = PermissionPlugin;
        run_before_tool_execute(&plugin, &mut step).await;
        apply_interaction_intents(&mut step);

        // "tools" is not an object -> tools.get(tool_id) returns None -> falls
        // back to default_behavior "allow"
        assert!(!step.tool_blocked());
        assert!(!step.tool_pending());
    }

    #[tokio::test]
    async fn test_permission_plugin_default_behavior_invalid_string() {
        // "default_behavior" is an unrecognized string — should fall back to Ask
        let fixture = TestFixture::new_with_state(
            json!({ "permissions": { "default_behavior": "invalid_value", "tools": {} } }),
        );
        let mut step = fixture.step(vec![]);

        let call = ToolCall::new("call_1", "any_tool", json!({}));
        step.tool = Some(ToolContext::new(&call));

        let plugin = PermissionPlugin;
        run_before_tool_execute(&plugin, &mut step).await;
        apply_interaction_intents(&mut step);

        // parse_behavior("invalid_value") returns None -> unwrap_or(Ask)
        assert!(step.tool_pending());
    }

    #[tokio::test]
    async fn test_permission_plugin_default_behavior_is_number() {
        // "default_behavior" is a number instead of string — should fall back to Ask
        let fixture = TestFixture::new_with_state(
            json!({ "permissions": { "default_behavior": 42, "tools": {} } }),
        );
        let mut step = fixture.step(vec![]);

        let call = ToolCall::new("call_1", "any_tool", json!({}));
        step.tool = Some(ToolContext::new(&call));

        let plugin = PermissionPlugin;
        run_before_tool_execute(&plugin, &mut step).await;
        apply_interaction_intents(&mut step);

        // as_str() on a number returns None -> parse_behavior not called -> unwrap_or(Ask)
        assert!(step.tool_pending());
    }

    #[tokio::test]
    async fn test_permission_plugin_tool_value_is_number() {
        // Tool permission value is a number — should fall back to default_behavior
        let fixture = TestFixture::new_with_state(
            json!({ "permissions": { "default_behavior": "allow", "tools": { "my_tool": 123 } } }),
        );
        let mut step = fixture.step(vec![]);

        let call = ToolCall::new("call_1", "my_tool", json!({}));
        step.tool = Some(ToolContext::new(&call));

        let plugin = PermissionPlugin;
        run_before_tool_execute(&plugin, &mut step).await;
        apply_interaction_intents(&mut step);

        // tools.get("my_tool") returns Some(123), as_str() returns None ->
        // parse_behavior not called -> falls to default_behavior "allow"
        assert!(!step.tool_blocked());
        assert!(!step.tool_pending());
    }

    #[tokio::test]
    async fn test_permission_plugin_permissions_is_array() {
        // "permissions" is an array instead of object — should fall back to Ask
        let fixture = TestFixture::new_with_state(json!({ "permissions": [1, 2, 3] }));
        let mut step = fixture.step(vec![]);

        let call = ToolCall::new("call_1", "any_tool", json!({}));
        step.tool = Some(ToolContext::new(&call));

        let plugin = PermissionPlugin;
        run_before_tool_execute(&plugin, &mut step).await;
        apply_interaction_intents(&mut step);

        // Array can't .get("tools") -> None -> falls to default check ->
        // Array can't .get("default_behavior") -> None -> unwrap_or(Ask)
        assert!(step.tool_pending());
    }

    #[tokio::test]
    async fn test_permission_resume_input_bypasses_ask() {
        let fixture = TestFixture::new_with_state(json!({
            "permissions": {
                "default_behavior": "ask",
                "tools": {}
            },
            "__tool_call_states": {
                "calls": {
                    "call_1": {
                        "call_id": "call_1",
                        "tool_name": "test_tool",
                        "arguments": {},
                        "status": "resuming",
                        "resume_token": "call_1",
                        "resume": {
                            "decision_id": "fc_call_1",
                            "action": "resume",
                            "result": true,
                            "updated_at": 1
                        },
                        "updated_at": 1
                    }
                }
            }
        }));
        let mut step = fixture.step(vec![]);
        let call = ToolCall::new("call_1", "test_tool", json!({}));
        step.tool = Some(ToolContext::new(&call));

        let plugin = PermissionPlugin;
        run_before_tool_execute(&plugin, &mut step).await;

        assert!(
            !step.tool_blocked(),
            "resume-approved call should be allowed"
        );
        assert!(
            !step.tool_pending(),
            "resume-approved call should not suspend again"
        );
    }
}
