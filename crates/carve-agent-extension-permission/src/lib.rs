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
//! use carve_agent::prelude::*;
//!
//! // In a tool implementation
//! async fn execute(&self, args: Value, ctx: &ContextAgentState) -> Result<ToolResult, ToolError> {
//!     // Allow follow-up tool after this one
//!     ctx.allow_tool("follow_up_tool");
//!     Ok(ToolResult::success("my_tool", json!({})))
//! }
//! ```

use async_trait::async_trait;
use carve_agent_contract::plugin::AgentPlugin;
use carve_agent_contract::runtime::Interaction;
use carve_agent_contract::context::ToolCallContext;
use carve_agent_contract::AgentState as ContextAgentState;
use carve_state::State;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::HashMap;

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
#[carve(path = "permissions")]
pub struct PermissionState {
    /// Default behavior for tools not explicitly configured.
    pub default_behavior: ToolPermissionBehavior,
    /// Per-tool permission overrides.
    pub tools: HashMap<String, ToolPermissionBehavior>,
}

/// Unified frontend ask tool name used for user interaction prompts.
pub const ASK_USER_TOOL_NAME: &str = "AskUserQuestion";

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

impl PermissionContextExt for ContextAgentState {
    fn allow_tool(&self, tool_id: impl Into<String>) {
        let state = self.state_of::<PermissionState>();
        state.tools_insert(tool_id.into(), ToolPermissionBehavior::Allow);
    }

    fn deny_tool(&self, tool_id: impl Into<String>) {
        let state = self.state_of::<PermissionState>();
        state.tools_insert(tool_id.into(), ToolPermissionBehavior::Deny);
    }

    fn ask_tool(&self, tool_id: impl Into<String>) {
        let state = self.state_of::<PermissionState>();
        state.tools_insert(tool_id.into(), ToolPermissionBehavior::Ask);
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
        state.set_default_behavior(behavior);
    }

    fn get_default_permission(&self) -> ToolPermissionBehavior {
        let state = self.state_of::<PermissionState>();
        state.default_behavior().ok().unwrap_or_default()
    }
}

impl PermissionContextExt for ToolCallContext<'_> {
    fn allow_tool(&self, tool_id: impl Into<String>) {
        let state = self.state_of::<PermissionState>();
        state.tools_insert(tool_id.into(), ToolPermissionBehavior::Allow);
    }

    fn deny_tool(&self, tool_id: impl Into<String>) {
        let state = self.state_of::<PermissionState>();
        state.tools_insert(tool_id.into(), ToolPermissionBehavior::Deny);
    }

    fn ask_tool(&self, tool_id: impl Into<String>) {
        let state = self.state_of::<PermissionState>();
        state.tools_insert(tool_id.into(), ToolPermissionBehavior::Ask);
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
        state.set_default_behavior(behavior);
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
/// - `Ask`: set tool pending interaction directly
pub struct PermissionPlugin;

#[async_trait]
impl AgentPlugin for PermissionPlugin {
    fn id(&self) -> &str {
        "permission"
    }

    async fn on_phase(
        &self,
        phase: carve_agent_contract::runtime::phase::Phase,
        step: &mut carve_agent_contract::runtime::phase::StepContext<'_>,
    ) {
        use carve_agent_contract::runtime::phase::Phase;

        if phase != Phase::BeforeToolExecute {
            return;
        }

        let Some(tool_id) = step.tool_name() else {
            return;
        };

        let permission = step.ctx().get_permission(tool_id);

        match permission {
            ToolPermissionBehavior::Allow => {
                // Allowed - do nothing
            }
            ToolPermissionBehavior::Deny => {
                step.block(format!("Tool '{}' is denied", tool_id));
            }
            ToolPermissionBehavior::Ask => {
                let origin_call_id = step.tool_call_id().unwrap_or_default().to_string();
                let origin_args = step.tool_args().cloned().unwrap_or_default();
                let question = format!("Allow tool '{}' to execute?", tool_id);
                // Create a pending interaction for confirmation
                let interaction = Interaction::new(format!("permission_{}", tool_id), "confirm")
                    .with_message(question.clone())
                    .with_parameters(json!({
                        "questions": [
                            {
                                "question": question,
                                "header": "Permission",
                                "options": [
                                    {
                                        "label": "Allow",
                                        "description": format!("Allow '{}' to run.", tool_id)
                                    },
                                    {
                                        "label": "Deny",
                                        "description": format!("Deny '{}' from running.", tool_id)
                                    }
                                ],
                                "multiSelect": false
                            }
                        ],
                        "ask_call_id": format!("permission_{}", tool_id),
                        "ask_user_tool": ASK_USER_TOOL_NAME,
                        "origin_call_id": origin_call_id,
                        "origin_call_name": tool_id,
                        "origin_tool_call": {
                            "id": origin_call_id,
                            "name": tool_id,
                            "arguments": origin_args
                        }
                    }));
                step.pending(interaction);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use carve_agent_contract::AgentState as ContextAgentState;
    use serde_json::json;

    fn apply_interaction_intents(
        _step: &mut carve_agent_contract::runtime::phase::StepContext<'_>,
    ) {
        // No-op: permission plugin now sets pending interaction directly.
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
        let doc = json!({
            "permissions": {
                "default_behavior": "deny",
                "tools": {
                    "recover_agent_run": "allow"
                }
            }
        });
        let ctx = ContextAgentState::new_transient(&doc, "call_1", "test");
        assert_eq!(
            ctx.get_permission("recover_agent_run"),
            ToolPermissionBehavior::Allow
        );
    }

    #[test]
    fn test_get_permission_falls_back_to_default() {
        let doc = json!({
            "permissions": {
                "default_behavior": "deny",
                "tools": {}
            }
        });
        let ctx = ContextAgentState::new_transient(&doc, "call_1", "test");
        assert_eq!(
            ctx.get_permission("unknown_tool"),
            ToolPermissionBehavior::Deny
        );
    }

    #[test]
    fn test_get_permission_missing_state_falls_back_to_ask() {
        let doc = json!({});
        let ctx = ContextAgentState::new_transient(&doc, "call_1", "test");
        assert_eq!(
            ctx.get_permission("recover_agent_run"),
            ToolPermissionBehavior::Ask
        );
    }

    #[test]
    fn test_allow_tool() {
        let doc = json!({
            "permissions": {
                "default_behavior": "ask",
                "tools": {}
            }
        });
        let ctx = ContextAgentState::new_transient(&doc, "call_1", "test");

        ctx.allow_tool("read_file");
        // Note: We can't verify get_permission() returns Allow because
        // Context collects ops that need to be applied to see the result.
        assert!(ctx.has_changes());
    }

    #[test]
    fn test_deny_tool() {
        let doc = json!({
            "permissions": {
                "default_behavior": "ask",
                "tools": {}
            }
        });
        let ctx = ContextAgentState::new_transient(&doc, "call_1", "test");

        ctx.deny_tool("delete_file");
        // Note: We can't verify get_permission() returns Deny because
        // Context collects ops that need to be applied to see the result.
        assert!(ctx.has_changes());
    }

    #[test]
    fn test_ask_tool() {
        let doc = json!({
            "permissions": {
                "default_behavior": "allow",
                "tools": {}
            }
        });
        let ctx = ContextAgentState::new_transient(&doc, "call_1", "test");

        ctx.ask_tool("write_file");
        // Note: We can't verify get_permission() returns Ask because
        // Context collects ops that need to be applied to see the result.
        assert!(ctx.has_changes());
    }

    #[test]
    fn test_get_permission_default() {
        let doc = json!({
            "permissions": {
                "default_behavior": "deny",
                "tools": {}
            }
        });
        let ctx = ContextAgentState::new_transient(&doc, "call_1", "test");

        // Tool not in tools map should return default
        assert_eq!(
            ctx.get_permission("unknown_tool"),
            ToolPermissionBehavior::Deny
        );
    }

    #[test]
    fn test_get_permission_override() {
        let doc = json!({
            "permissions": {
                "default_behavior": "deny",
                "tools": { "special_tool": "allow" }
            }
        });
        let ctx = ContextAgentState::new_transient(&doc, "call_1", "test");

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
        let doc = json!({
            "permissions": {
                "default_behavior": "ask",
                "tools": {}
            }
        });
        let ctx = ContextAgentState::new_transient(&doc, "call_1", "test");

        ctx.set_default_permission(ToolPermissionBehavior::Allow);
        // Note: We can't verify get_default_permission() returns Allow because
        // Context collects ops that need to be applied to see the result.
        assert!(ctx.has_changes());
    }

    #[test]
    fn test_permission_plugin_id() {
        let plugin = PermissionPlugin;
        assert_eq!(plugin.id(), "permission");
    }

    #[tokio::test]
    async fn test_permission_plugin_allow() {
        let doc = json!({ "permissions": { "default_behavior": "allow", "tools": {} } });
        let ctx = ContextAgentState::new_transient(&doc, "test", "test");
        use carve_agent_contract::runtime::phase::{Phase, StepContext, ToolContext};
        use carve_agent_contract::state::AgentState;
        use carve_agent_contract::state::ToolCall;

        let thread = AgentState::with_initial_state(
            "test",
            json!({ "permissions": { "default_behavior": "allow", "tools": {} } }),
        );
        let mut step = StepContext::new(&thread, vec![]);

        let call = ToolCall::new("call_1", "any_tool", json!({}));
        step.tool = Some(ToolContext::new(&call));

        let plugin = PermissionPlugin;
        plugin
            .on_phase(Phase::BeforeToolExecute, &mut step, &ctx)
            .await;
        apply_interaction_intents(&mut step);

        assert!(!step.tool_blocked());
        assert!(!step.tool_pending());
    }

    #[tokio::test]
    async fn test_permission_plugin_deny() {
        let doc = json!({ "permissions": { "default_behavior": "deny", "tools": {} } });
        let ctx = ContextAgentState::new_transient(&doc, "test", "test");
        use carve_agent_contract::runtime::phase::{Phase, StepContext, ToolContext};
        use carve_agent_contract::state::AgentState;
        use carve_agent_contract::state::ToolCall;

        let thread = AgentState::with_initial_state(
            "test",
            json!({ "permissions": { "default_behavior": "deny", "tools": {} } }),
        );
        let mut step = StepContext::new(&thread, vec![]);

        let call = ToolCall::new("call_1", "any_tool", json!({}));
        step.tool = Some(ToolContext::new(&call));

        let plugin = PermissionPlugin;
        plugin
            .on_phase(Phase::BeforeToolExecute, &mut step, &ctx)
            .await;
        apply_interaction_intents(&mut step);

        assert!(step.tool_blocked());
    }

    #[tokio::test]
    async fn test_permission_plugin_ask() {
        let doc = json!({ "permissions": { "default_behavior": "ask", "tools": {} } });
        let ctx = ContextAgentState::new_transient(&doc, "test", "test");
        use carve_agent_contract::runtime::phase::{Phase, StepContext, ToolContext};
        use carve_agent_contract::state::AgentState;
        use carve_agent_contract::state::ToolCall;

        let thread = AgentState::with_initial_state(
            "test",
            json!({ "permissions": { "default_behavior": "ask", "tools": {} } }),
        );
        let mut step = StepContext::new(&thread, vec![]);

        let call = ToolCall::new("call_1", "test_tool", json!({}));
        step.tool = Some(ToolContext::new(&call));

        let plugin = PermissionPlugin;
        plugin
            .on_phase(Phase::BeforeToolExecute, &mut step, &ctx)
            .await;
        apply_interaction_intents(&mut step);

        assert!(step.tool_pending());
        let interaction = step
            .tool
            .as_ref()
            .and_then(|t| t.pending_interaction.as_ref())
            .expect("pending interaction should exist");
        assert_eq!(interaction.action, "confirm");
        assert!(interaction.parameters.get("questions").is_some());
        assert_eq!(interaction.parameters["ask_user_tool"], ASK_USER_TOOL_NAME);
        assert_eq!(interaction.parameters["origin_call_id"], "call_1");
        assert_eq!(interaction.parameters["origin_call_name"], "test_tool");
        assert_eq!(interaction.parameters["origin_tool_call"]["id"], "call_1");
        assert_eq!(
            interaction.parameters["origin_tool_call"]["name"],
            "test_tool"
        );
    }

    #[test]
    fn test_get_default_permission() {
        let doc = json!({
            "permissions": {
                "default_behavior": "allow",
                "tools": {}
            }
        });
        let ctx = ContextAgentState::new_transient(&doc, "call_1", "test");

        // Should read default_behavior from state
        let default = ctx.get_default_permission();
        assert_eq!(default, ToolPermissionBehavior::Allow);
    }

    #[test]
    fn test_get_default_permission_deny() {
        let doc = json!({
            "permissions": {
                "default_behavior": "deny",
                "tools": {}
            }
        });
        let ctx = ContextAgentState::new_transient(&doc, "call_1", "test");

        let default = ctx.get_default_permission();
        assert_eq!(default, ToolPermissionBehavior::Deny);
    }

    #[test]
    fn test_get_default_permission_fallback() {
        // When state is missing, should return default (Ask)
        let doc = json!({});
        let ctx = ContextAgentState::new_transient(&doc, "call_1", "test");

        let default = ctx.get_default_permission();
        assert_eq!(default, ToolPermissionBehavior::Ask);
    }

    #[tokio::test]
    async fn test_permission_plugin_tool_specific_allow() {
        let doc = json!({ "permissions": { "default_behavior": "deny", "tools": { "allowed_tool": "allow" } } });
        let ctx = ContextAgentState::new_transient(&doc, "test", "test");
        use carve_agent_contract::runtime::phase::{Phase, StepContext, ToolContext};
        use carve_agent_contract::state::AgentState;
        use carve_agent_contract::state::ToolCall;

        let thread = AgentState::with_initial_state(
            "test",
            json!({ "permissions": { "default_behavior": "deny", "tools": { "allowed_tool": "allow" } } }),
        );
        let mut step = StepContext::new(&thread, vec![]);

        let call = ToolCall::new("call_1", "allowed_tool", json!({}));
        step.tool = Some(ToolContext::new(&call));

        let plugin = PermissionPlugin;
        plugin
            .on_phase(Phase::BeforeToolExecute, &mut step, &ctx)
            .await;
        apply_interaction_intents(&mut step);

        assert!(!step.tool_blocked());
    }

    #[tokio::test]
    async fn test_permission_plugin_tool_specific_deny() {
        let doc = json!({ "permissions": { "default_behavior": "allow", "tools": { "denied_tool": "deny" } } });
        let ctx = ContextAgentState::new_transient(&doc, "test", "test");
        use carve_agent_contract::runtime::phase::{Phase, StepContext, ToolContext};
        use carve_agent_contract::state::AgentState;
        use carve_agent_contract::state::ToolCall;

        let thread = AgentState::with_initial_state(
            "test",
            json!({ "permissions": { "default_behavior": "allow", "tools": { "denied_tool": "deny" } } }),
        );
        let mut step = StepContext::new(&thread, vec![]);

        let call = ToolCall::new("call_1", "denied_tool", json!({}));
        step.tool = Some(ToolContext::new(&call));

        let plugin = PermissionPlugin;
        plugin
            .on_phase(Phase::BeforeToolExecute, &mut step, &ctx)
            .await;
        apply_interaction_intents(&mut step);

        assert!(step.tool_blocked());
    }

    #[tokio::test]
    async fn test_permission_plugin_tool_specific_ask() {
        let doc = json!({ "permissions": { "default_behavior": "allow", "tools": { "ask_tool": "ask" } } });
        let ctx = ContextAgentState::new_transient(&doc, "test", "test");
        use carve_agent_contract::runtime::phase::{Phase, StepContext, ToolContext};
        use carve_agent_contract::state::AgentState;
        use carve_agent_contract::state::ToolCall;

        let thread = AgentState::with_initial_state(
            "test",
            json!({ "permissions": { "default_behavior": "allow", "tools": { "ask_tool": "ask" } } }),
        );
        let mut step = StepContext::new(&thread, vec![]);

        let call = ToolCall::new("call_1", "ask_tool", json!({}));
        step.tool = Some(ToolContext::new(&call));

        let plugin = PermissionPlugin;
        plugin
            .on_phase(Phase::BeforeToolExecute, &mut step, &ctx)
            .await;
        apply_interaction_intents(&mut step);

        assert!(step.tool_pending());
    }

    #[tokio::test]
    async fn test_permission_plugin_invalid_tool_behavior() {
        let doc = json!({ "permissions": { "default_behavior": "allow", "tools": { "invalid_tool": "invalid_behavior" } } });
        let ctx = ContextAgentState::new_transient(&doc, "test", "test");
        use carve_agent_contract::runtime::phase::{Phase, StepContext, ToolContext};
        use carve_agent_contract::state::AgentState;
        use carve_agent_contract::state::ToolCall;

        let thread = AgentState::with_initial_state(
            "test",
            json!({ "permissions": { "default_behavior": "allow", "tools": { "invalid_tool": "invalid_behavior" } } }),
        );
        let mut step = StepContext::new(&thread, vec![]);

        let call = ToolCall::new("call_1", "invalid_tool", json!({}));
        step.tool = Some(ToolContext::new(&call));

        let plugin = PermissionPlugin;
        plugin
            .on_phase(Phase::BeforeToolExecute, &mut step, &ctx)
            .await;
        apply_interaction_intents(&mut step);

        // Should fall back to default "allow" behavior
        assert!(!step.tool_blocked());
        assert!(!step.tool_pending());
    }

    #[tokio::test]
    async fn test_permission_plugin_invalid_default_behavior() {
        let doc = json!({ "permissions": { "default_behavior": "invalid_default", "tools": {} } });
        let ctx = ContextAgentState::new_transient(&doc, "test", "test");
        use carve_agent_contract::runtime::phase::{Phase, StepContext, ToolContext};
        use carve_agent_contract::state::AgentState;
        use carve_agent_contract::state::ToolCall;

        let thread = AgentState::with_initial_state(
            "test",
            json!({ "permissions": { "default_behavior": "invalid_default", "tools": {} } }),
        );
        let mut step = StepContext::new(&thread, vec![]);

        let call = ToolCall::new("call_1", "any_tool", json!({}));
        step.tool = Some(ToolContext::new(&call));

        let plugin = PermissionPlugin;
        plugin
            .on_phase(Phase::BeforeToolExecute, &mut step, &ctx)
            .await;
        apply_interaction_intents(&mut step);

        // Should fall back to Ask behavior
        assert!(step.tool_pending());
    }

    #[tokio::test]
    async fn test_permission_plugin_no_state() {
        let doc = json!({});
        let ctx = ContextAgentState::new_transient(&doc, "test", "test");
        use carve_agent_contract::runtime::phase::{Phase, StepContext, ToolContext};
        use carve_agent_contract::state::AgentState;
        use carve_agent_contract::state::ToolCall;

        // AgentState with no permission state at all — should default to Ask
        let thread = AgentState::new("test");
        let mut step = StepContext::new(&thread, vec![]);

        let call = ToolCall::new("call_1", "any_tool", json!({}));
        step.tool = Some(ToolContext::new(&call));

        let plugin = PermissionPlugin;
        plugin
            .on_phase(Phase::BeforeToolExecute, &mut step, &ctx)
            .await;
        apply_interaction_intents(&mut step);

        assert!(step.tool_pending());
    }

    // ========================================================================
    // Corrupted / unexpected state shape fallback tests
    // ========================================================================

    #[tokio::test]
    async fn test_permission_plugin_tools_is_string_not_object() {
        let doc = json!({ "permissions": { "default_behavior": "allow", "tools": "corrupted" } });
        let ctx = ContextAgentState::new_transient(&doc, "test", "test");
        use carve_agent_contract::runtime::phase::{Phase, StepContext, ToolContext};
        use carve_agent_contract::state::AgentState;
        use carve_agent_contract::state::ToolCall;

        // "tools" is a string instead of an object — should not panic,
        // falls back to default_behavior.
        let thread = AgentState::with_initial_state(
            "test",
            json!({ "permissions": { "default_behavior": "allow", "tools": "corrupted" } }),
        );
        let mut step = StepContext::new(&thread, vec![]);

        let call = ToolCall::new("call_1", "any_tool", json!({}));
        step.tool = Some(ToolContext::new(&call));

        let plugin = PermissionPlugin;
        plugin
            .on_phase(Phase::BeforeToolExecute, &mut step, &ctx)
            .await;
        apply_interaction_intents(&mut step);

        // "tools" is not an object → tools.get(tool_id) returns None → falls
        // back to default_behavior "allow"
        assert!(!step.tool_blocked());
        assert!(!step.tool_pending());
    }

    #[tokio::test]
    async fn test_permission_plugin_default_behavior_invalid_string() {
        let doc = json!({ "permissions": { "default_behavior": "invalid_value", "tools": {} } });
        let ctx = ContextAgentState::new_transient(&doc, "test", "test");
        use carve_agent_contract::runtime::phase::{Phase, StepContext, ToolContext};
        use carve_agent_contract::state::AgentState;
        use carve_agent_contract::state::ToolCall;

        // "default_behavior" is an unrecognized string — should fall back to Ask
        let thread = AgentState::with_initial_state(
            "test",
            json!({ "permissions": { "default_behavior": "invalid_value", "tools": {} } }),
        );
        let mut step = StepContext::new(&thread, vec![]);

        let call = ToolCall::new("call_1", "any_tool", json!({}));
        step.tool = Some(ToolContext::new(&call));

        let plugin = PermissionPlugin;
        plugin
            .on_phase(Phase::BeforeToolExecute, &mut step, &ctx)
            .await;
        apply_interaction_intents(&mut step);

        // parse_behavior("invalid_value") returns None → unwrap_or(Ask)
        assert!(step.tool_pending());
    }

    #[tokio::test]
    async fn test_permission_plugin_default_behavior_is_number() {
        let doc = json!({ "permissions": { "default_behavior": 42, "tools": {} } });
        let ctx = ContextAgentState::new_transient(&doc, "test", "test");
        use carve_agent_contract::runtime::phase::{Phase, StepContext, ToolContext};
        use carve_agent_contract::state::AgentState;
        use carve_agent_contract::state::ToolCall;

        // "default_behavior" is a number instead of string — should fall back to Ask
        let thread = AgentState::with_initial_state(
            "test",
            json!({ "permissions": { "default_behavior": 42, "tools": {} } }),
        );
        let mut step = StepContext::new(&thread, vec![]);

        let call = ToolCall::new("call_1", "any_tool", json!({}));
        step.tool = Some(ToolContext::new(&call));

        let plugin = PermissionPlugin;
        plugin
            .on_phase(Phase::BeforeToolExecute, &mut step, &ctx)
            .await;
        apply_interaction_intents(&mut step);

        // as_str() on a number returns None → parse_behavior not called → unwrap_or(Ask)
        assert!(step.tool_pending());
    }

    #[tokio::test]
    async fn test_permission_plugin_tool_value_is_number() {
        let doc =
            json!({ "permissions": { "default_behavior": "allow", "tools": { "my_tool": 123 } } });
        let ctx = ContextAgentState::new_transient(&doc, "test", "test");
        use carve_agent_contract::runtime::phase::{Phase, StepContext, ToolContext};
        use carve_agent_contract::state::AgentState;
        use carve_agent_contract::state::ToolCall;

        // Tool permission value is a number — should fall back to default_behavior
        let thread = AgentState::with_initial_state(
            "test",
            json!({ "permissions": { "default_behavior": "allow", "tools": { "my_tool": 123 } } }),
        );
        let mut step = StepContext::new(&thread, vec![]);

        let call = ToolCall::new("call_1", "my_tool", json!({}));
        step.tool = Some(ToolContext::new(&call));

        let plugin = PermissionPlugin;
        plugin
            .on_phase(Phase::BeforeToolExecute, &mut step, &ctx)
            .await;
        apply_interaction_intents(&mut step);

        // tools.get("my_tool") returns Some(123), as_str() returns None →
        // parse_behavior not called → falls to default_behavior "allow"
        assert!(!step.tool_blocked());
        assert!(!step.tool_pending());
    }

    #[tokio::test]
    async fn test_permission_plugin_permissions_is_array() {
        let doc = json!({ "permissions": [1, 2, 3] });
        let ctx = ContextAgentState::new_transient(&doc, "test", "test");
        use carve_agent_contract::runtime::phase::{Phase, StepContext, ToolContext};
        use carve_agent_contract::state::AgentState;
        use carve_agent_contract::state::ToolCall;

        // "permissions" is an array instead of object — should fall back to Ask
        let thread = AgentState::with_initial_state("test", json!({ "permissions": [1, 2, 3] }));
        let mut step = StepContext::new(&thread, vec![]);

        let call = ToolCall::new("call_1", "any_tool", json!({}));
        step.tool = Some(ToolContext::new(&call));

        let plugin = PermissionPlugin;
        plugin
            .on_phase(Phase::BeforeToolExecute, &mut step, &ctx)
            .await;
        apply_interaction_intents(&mut step);

        // Array can't .get("tools") → None → falls to default check →
        // Array can't .get("default_behavior") → None → unwrap_or(Ask)
        assert!(step.tool_pending());
    }
}
