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
//! async fn execute(&self, args: Value, ctx: &Context<'_>) -> Result<ToolResult, ToolError> {
//!     // Allow follow-up tool after this one
//!     ctx.allow_tool("follow_up_tool");
//!     Ok(ToolResult::success("my_tool", json!({})))
//! }
//! ```

use crate::plugin::AgentPlugin;
use crate::state_types::{Interaction, ToolPermissionBehavior};
use async_trait::async_trait;
use carve_state::Context;
use carve_state_derive::State;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::HashMap;

/// State path for permission state.
pub const PERMISSION_STATE_PATH: &str = "permissions";

/// Permission state stored in session state.
#[derive(Debug, Clone, Serialize, Deserialize, State)]
pub struct PermissionState {
    /// Default behavior for tools not explicitly configured.
    pub default_behavior: ToolPermissionBehavior,
    /// Per-tool permission overrides.
    pub tools: HashMap<String, ToolPermissionBehavior>,
}

impl Default for PermissionState {
    fn default() -> Self {
        Self {
            default_behavior: ToolPermissionBehavior::Ask,
            tools: HashMap::new(),
        }
    }
}

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

impl PermissionContextExt for Context<'_> {
    fn allow_tool(&self, tool_id: impl Into<String>) {
        let state = self.state::<PermissionState>(PERMISSION_STATE_PATH);
        state.tools_insert(tool_id.into(), ToolPermissionBehavior::Allow);
    }

    fn deny_tool(&self, tool_id: impl Into<String>) {
        let state = self.state::<PermissionState>(PERMISSION_STATE_PATH);
        state.tools_insert(tool_id.into(), ToolPermissionBehavior::Deny);
    }

    fn ask_tool(&self, tool_id: impl Into<String>) {
        let state = self.state::<PermissionState>(PERMISSION_STATE_PATH);
        state.tools_insert(tool_id.into(), ToolPermissionBehavior::Ask);
    }

    fn get_permission(&self, tool_id: &str) -> ToolPermissionBehavior {
        let state = self.state::<PermissionState>(PERMISSION_STATE_PATH);
        state
            .tools()
            .ok()
            .and_then(|tools| tools.get(tool_id).copied())
            .unwrap_or_else(|| state.default_behavior().ok().unwrap_or_default())
    }

    fn set_default_permission(&self, behavior: ToolPermissionBehavior) {
        let state = self.state::<PermissionState>(PERMISSION_STATE_PATH);
        state.set_default_behavior(behavior);
    }

    fn get_default_permission(&self) -> ToolPermissionBehavior {
        let state = self.state::<PermissionState>(PERMISSION_STATE_PATH);
        state.default_behavior().ok().unwrap_or_default()
    }
}

/// Plugin that enforces tool permissions.
///
/// This plugin checks permissions in `before_tool_execute` and:
/// - Allows execution if permission is `Allow`
/// - Blocks execution if permission is `Deny`
/// - Creates a pending interaction if permission is `Ask`
pub struct PermissionPlugin;

#[async_trait]
impl AgentPlugin for PermissionPlugin {
    fn id(&self) -> &str {
        "permission"
    }

    async fn on_phase(&self, phase: crate::phase::Phase, step: &mut crate::phase::StepContext<'_>) {
        use crate::phase::Phase;

        if phase != Phase::BeforeToolExecute {
            return;
        }

        let Some(tool_id) = step.tool_name() else {
            return;
        };

        // Read permission state from session state
        let permission_state = step
            .session
            .rebuild_state()
            .ok()
            .and_then(|s| s.get(PERMISSION_STATE_PATH).cloned());

        let permission = permission_state
            .as_ref()
            .and_then(|state| {
                state
                    .get("tools")
                    .and_then(|tools| tools.get(tool_id))
                    .and_then(|v| v.as_str())
                    .and_then(parse_behavior)
            })
            .unwrap_or_else(|| {
                permission_state
                    .as_ref()
                    .and_then(|state| {
                        state
                            .get("default_behavior")
                            .and_then(|v| v.as_str())
                            .and_then(parse_behavior)
                    })
                    .unwrap_or(ToolPermissionBehavior::Ask)
            });

        match permission {
            ToolPermissionBehavior::Allow => {
                // Allowed - do nothing
            }
            ToolPermissionBehavior::Deny => {
                step.block(format!("Tool '{}' is denied", tool_id));
            }
            ToolPermissionBehavior::Ask => {
                // Create a pending interaction for confirmation
                if !step.tool_pending() {
                    let interaction =
                        Interaction::new(format!("permission_{}", tool_id), "confirm")
                            .with_message(format!("Allow tool '{}' to execute?", tool_id))
                            .with_parameters(json!({ "tool_id": tool_id }));

                    step.pending(interaction);
                }
            }
        }
    }
}

/// Parse a behavior string into a ToolPermissionBehavior.
fn parse_behavior(s: &str) -> Option<ToolPermissionBehavior> {
    match s {
        "allow" => Some(ToolPermissionBehavior::Allow),
        "deny" => Some(ToolPermissionBehavior::Deny),
        "ask" => Some(ToolPermissionBehavior::Ask),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

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
    fn test_allow_tool() {
        let doc = json!({
            "permissions": {
                "default_behavior": "ask",
                "tools": {}
            }
        });
        let ctx = Context::new(&doc, "call_1", "test");

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
        let ctx = Context::new(&doc, "call_1", "test");

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
        let ctx = Context::new(&doc, "call_1", "test");

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
        let ctx = Context::new(&doc, "call_1", "test");

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
        let ctx = Context::new(&doc, "call_1", "test");

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
        let ctx = Context::new(&doc, "call_1", "test");

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

    #[test]
    fn test_permission_plugin_no_initial_data() {
        let plugin = PermissionPlugin;
        assert!(plugin.initial_scratchpad().is_none());
    }

    #[tokio::test]
    async fn test_permission_plugin_allow() {
        use crate::phase::{Phase, StepContext, ToolContext};
        use crate::session::Session;
        use crate::types::ToolCall;

        let session = Session::with_initial_state(
            "test",
            json!({ "permissions": { "default_behavior": "allow", "tools": {} } }),
        );
        let mut step = StepContext::new(&session, vec![]);

        let call = ToolCall::new("call_1", "any_tool", json!({}));
        step.tool = Some(ToolContext::new(&call));

        let plugin = PermissionPlugin;
        plugin.on_phase(Phase::BeforeToolExecute, &mut step).await;

        assert!(!step.tool_blocked());
        assert!(!step.tool_pending());
    }

    #[tokio::test]
    async fn test_permission_plugin_deny() {
        use crate::phase::{Phase, StepContext, ToolContext};
        use crate::session::Session;
        use crate::types::ToolCall;

        let session = Session::with_initial_state(
            "test",
            json!({ "permissions": { "default_behavior": "deny", "tools": {} } }),
        );
        let mut step = StepContext::new(&session, vec![]);

        let call = ToolCall::new("call_1", "any_tool", json!({}));
        step.tool = Some(ToolContext::new(&call));

        let plugin = PermissionPlugin;
        plugin.on_phase(Phase::BeforeToolExecute, &mut step).await;

        assert!(step.tool_blocked());
    }

    #[tokio::test]
    async fn test_permission_plugin_ask() {
        use crate::phase::{Phase, StepContext, ToolContext};
        use crate::session::Session;
        use crate::types::ToolCall;

        let session = Session::with_initial_state(
            "test",
            json!({ "permissions": { "default_behavior": "ask", "tools": {} } }),
        );
        let mut step = StepContext::new(&session, vec![]);

        let call = ToolCall::new("call_1", "test_tool", json!({}));
        step.tool = Some(ToolContext::new(&call));

        let plugin = PermissionPlugin;
        plugin.on_phase(Phase::BeforeToolExecute, &mut step).await;

        assert!(step.tool_pending());
    }

    #[test]
    fn test_get_default_permission() {
        let doc = json!({
            "permissions": {
                "default_behavior": "allow",
                "tools": {}
            }
        });
        let ctx = Context::new(&doc, "call_1", "test");

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
        let ctx = Context::new(&doc, "call_1", "test");

        let default = ctx.get_default_permission();
        assert_eq!(default, ToolPermissionBehavior::Deny);
    }

    #[test]
    fn test_get_default_permission_fallback() {
        // When state is missing, should return default (Ask)
        let doc = json!({});
        let ctx = Context::new(&doc, "call_1", "test");

        let default = ctx.get_default_permission();
        assert_eq!(default, ToolPermissionBehavior::Ask);
    }

    #[tokio::test]
    async fn test_permission_plugin_tool_specific_allow() {
        use crate::phase::{Phase, StepContext, ToolContext};
        use crate::session::Session;
        use crate::types::ToolCall;

        let session = Session::with_initial_state(
            "test",
            json!({ "permissions": { "default_behavior": "deny", "tools": { "allowed_tool": "allow" } } }),
        );
        let mut step = StepContext::new(&session, vec![]);

        let call = ToolCall::new("call_1", "allowed_tool", json!({}));
        step.tool = Some(ToolContext::new(&call));

        let plugin = PermissionPlugin;
        plugin.on_phase(Phase::BeforeToolExecute, &mut step).await;

        assert!(!step.tool_blocked());
    }

    #[tokio::test]
    async fn test_permission_plugin_tool_specific_deny() {
        use crate::phase::{Phase, StepContext, ToolContext};
        use crate::session::Session;
        use crate::types::ToolCall;

        let session = Session::with_initial_state(
            "test",
            json!({ "permissions": { "default_behavior": "allow", "tools": { "denied_tool": "deny" } } }),
        );
        let mut step = StepContext::new(&session, vec![]);

        let call = ToolCall::new("call_1", "denied_tool", json!({}));
        step.tool = Some(ToolContext::new(&call));

        let plugin = PermissionPlugin;
        plugin.on_phase(Phase::BeforeToolExecute, &mut step).await;

        assert!(step.tool_blocked());
    }

    #[tokio::test]
    async fn test_permission_plugin_tool_specific_ask() {
        use crate::phase::{Phase, StepContext, ToolContext};
        use crate::session::Session;
        use crate::types::ToolCall;

        let session = Session::with_initial_state(
            "test",
            json!({ "permissions": { "default_behavior": "allow", "tools": { "ask_tool": "ask" } } }),
        );
        let mut step = StepContext::new(&session, vec![]);

        let call = ToolCall::new("call_1", "ask_tool", json!({}));
        step.tool = Some(ToolContext::new(&call));

        let plugin = PermissionPlugin;
        plugin.on_phase(Phase::BeforeToolExecute, &mut step).await;

        assert!(step.tool_pending());
    }

    #[tokio::test]
    async fn test_permission_plugin_invalid_tool_behavior() {
        use crate::phase::{Phase, StepContext, ToolContext};
        use crate::session::Session;
        use crate::types::ToolCall;

        let session = Session::with_initial_state(
            "test",
            json!({ "permissions": { "default_behavior": "allow", "tools": { "invalid_tool": "invalid_behavior" } } }),
        );
        let mut step = StepContext::new(&session, vec![]);

        let call = ToolCall::new("call_1", "invalid_tool", json!({}));
        step.tool = Some(ToolContext::new(&call));

        let plugin = PermissionPlugin;
        plugin.on_phase(Phase::BeforeToolExecute, &mut step).await;

        // Should fall back to default "allow" behavior
        assert!(!step.tool_blocked());
        assert!(!step.tool_pending());
    }

    #[tokio::test]
    async fn test_permission_plugin_invalid_default_behavior() {
        use crate::phase::{Phase, StepContext, ToolContext};
        use crate::session::Session;
        use crate::types::ToolCall;

        let session = Session::with_initial_state(
            "test",
            json!({ "permissions": { "default_behavior": "invalid_default", "tools": {} } }),
        );
        let mut step = StepContext::new(&session, vec![]);

        let call = ToolCall::new("call_1", "any_tool", json!({}));
        step.tool = Some(ToolContext::new(&call));

        let plugin = PermissionPlugin;
        plugin.on_phase(Phase::BeforeToolExecute, &mut step).await;

        // Should fall back to Ask behavior
        assert!(step.tool_pending());
    }

    #[tokio::test]
    async fn test_permission_plugin_no_state() {
        use crate::phase::{Phase, StepContext, ToolContext};
        use crate::session::Session;
        use crate::types::ToolCall;

        // Session with no permission state at all — should default to Ask
        let session = Session::new("test");
        let mut step = StepContext::new(&session, vec![]);

        let call = ToolCall::new("call_1", "any_tool", json!({}));
        step.tool = Some(ToolContext::new(&call));

        let plugin = PermissionPlugin;
        plugin.on_phase(Phase::BeforeToolExecute, &mut step).await;

        assert!(step.tool_pending());
    }

    // ========================================================================
    // Corrupted / unexpected state shape fallback tests
    // ========================================================================

    #[tokio::test]
    async fn test_permission_plugin_tools_is_string_not_object() {
        use crate::phase::{Phase, StepContext, ToolContext};
        use crate::session::Session;
        use crate::types::ToolCall;

        // "tools" is a string instead of an object — should not panic,
        // falls back to default_behavior.
        let session = Session::with_initial_state(
            "test",
            json!({ "permissions": { "default_behavior": "allow", "tools": "corrupted" } }),
        );
        let mut step = StepContext::new(&session, vec![]);

        let call = ToolCall::new("call_1", "any_tool", json!({}));
        step.tool = Some(ToolContext::new(&call));

        let plugin = PermissionPlugin;
        plugin.on_phase(Phase::BeforeToolExecute, &mut step).await;

        // "tools" is not an object → tools.get(tool_id) returns None → falls
        // back to default_behavior "allow"
        assert!(!step.tool_blocked());
        assert!(!step.tool_pending());
    }

    #[tokio::test]
    async fn test_permission_plugin_default_behavior_invalid_string() {
        use crate::phase::{Phase, StepContext, ToolContext};
        use crate::session::Session;
        use crate::types::ToolCall;

        // "default_behavior" is an unrecognized string — should fall back to Ask
        let session = Session::with_initial_state(
            "test",
            json!({ "permissions": { "default_behavior": "invalid_value", "tools": {} } }),
        );
        let mut step = StepContext::new(&session, vec![]);

        let call = ToolCall::new("call_1", "any_tool", json!({}));
        step.tool = Some(ToolContext::new(&call));

        let plugin = PermissionPlugin;
        plugin.on_phase(Phase::BeforeToolExecute, &mut step).await;

        // parse_behavior("invalid_value") returns None → unwrap_or(Ask)
        assert!(step.tool_pending());
    }

    #[tokio::test]
    async fn test_permission_plugin_default_behavior_is_number() {
        use crate::phase::{Phase, StepContext, ToolContext};
        use crate::session::Session;
        use crate::types::ToolCall;

        // "default_behavior" is a number instead of string — should fall back to Ask
        let session = Session::with_initial_state(
            "test",
            json!({ "permissions": { "default_behavior": 42, "tools": {} } }),
        );
        let mut step = StepContext::new(&session, vec![]);

        let call = ToolCall::new("call_1", "any_tool", json!({}));
        step.tool = Some(ToolContext::new(&call));

        let plugin = PermissionPlugin;
        plugin.on_phase(Phase::BeforeToolExecute, &mut step).await;

        // as_str() on a number returns None → parse_behavior not called → unwrap_or(Ask)
        assert!(step.tool_pending());
    }

    #[tokio::test]
    async fn test_permission_plugin_tool_value_is_number() {
        use crate::phase::{Phase, StepContext, ToolContext};
        use crate::session::Session;
        use crate::types::ToolCall;

        // Tool permission value is a number — should fall back to default_behavior
        let session = Session::with_initial_state(
            "test",
            json!({ "permissions": { "default_behavior": "allow", "tools": { "my_tool": 123 } } }),
        );
        let mut step = StepContext::new(&session, vec![]);

        let call = ToolCall::new("call_1", "my_tool", json!({}));
        step.tool = Some(ToolContext::new(&call));

        let plugin = PermissionPlugin;
        plugin.on_phase(Phase::BeforeToolExecute, &mut step).await;

        // tools.get("my_tool") returns Some(123), as_str() returns None →
        // parse_behavior not called → falls to default_behavior "allow"
        assert!(!step.tool_blocked());
        assert!(!step.tool_pending());
    }

    #[tokio::test]
    async fn test_permission_plugin_permissions_is_array() {
        use crate::phase::{Phase, StepContext, ToolContext};
        use crate::session::Session;
        use crate::types::ToolCall;

        // "permissions" is an array instead of object — should fall back to Ask
        let session = Session::with_initial_state("test", json!({ "permissions": [1, 2, 3] }));
        let mut step = StepContext::new(&session, vec![]);

        let call = ToolCall::new("call_1", "any_tool", json!({}));
        step.tool = Some(ToolContext::new(&call));

        let plugin = PermissionPlugin;
        plugin.on_phase(Phase::BeforeToolExecute, &mut step).await;

        // Array can't .get("tools") → None → falls to default check →
        // Array can't .get("default_behavior") → None → unwrap_or(Ask)
        assert!(step.tool_pending());
    }
}
