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
use crate::plugins::execution::ExecutionContextExt;
use crate::state_types::{Interaction, ToolPermissionBehavior};
use async_trait::async_trait;
use carve_state::Context;
use carve_state_derive::State;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
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

    fn initial_state(&self) -> Option<(&'static str, Value)> {
        Some((
            PERMISSION_STATE_PATH,
            json!({
                "default_behavior": "ask",
                "tools": {}
            }),
        ))
    }

    async fn before_tool_execute(&self, ctx: &Context<'_>, tool_id: &str, _args: &Value) {
        let permission = ctx.get_permission(tool_id);

        match permission {
            ToolPermissionBehavior::Allow => {
                // Allowed - do nothing
            }
            ToolPermissionBehavior::Deny => {
                ctx.block(format!("Tool '{}' is denied", tool_id));
            }
            ToolPermissionBehavior::Ask => {
                // Create a pending interaction for confirmation
                let interaction = Interaction::confirm(
                    format!("permission_{}", tool_id),
                    format!("Allow tool '{}' to execute?", tool_id),
                )
                .with_metadata("tool_id", json!(tool_id));

                ctx.pending(interaction);
            }
        }
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
        state.tools.insert("read".to_string(), ToolPermissionBehavior::Allow);

        let json = serde_json::to_string(&state).unwrap();
        let parsed: PermissionState = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.tools.get("read"), Some(&ToolPermissionBehavior::Allow));
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
        assert_eq!(ctx.get_permission("unknown_tool"), ToolPermissionBehavior::Deny);
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

        assert_eq!(ctx.get_permission("special_tool"), ToolPermissionBehavior::Allow);
        assert_eq!(ctx.get_permission("other_tool"), ToolPermissionBehavior::Deny);
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
    fn test_permission_plugin_initial_state() {
        let plugin = PermissionPlugin;
        let state = plugin.initial_state();

        assert!(state.is_some());
        let (path, value) = state.unwrap();
        assert_eq!(path, "permissions");
        assert_eq!(value["default_behavior"], "ask");
    }

    #[tokio::test]
    async fn test_permission_plugin_allow() {
        let doc = json!({
            "permissions": {
                "default_behavior": "allow",
                "tools": {}
            },
            "execution": { "blocked": null, "pending": null }
        });
        let ctx = Context::new(&doc, "call_1", "test");
        let plugin = PermissionPlugin;

        plugin.before_tool_execute(&ctx, "any_tool", &json!({})).await;

        // Should not block or pending - no changes made
        assert!(!ctx.has_changes());
    }

    #[tokio::test]
    async fn test_permission_plugin_deny() {
        let doc = json!({
            "permissions": {
                "default_behavior": "deny",
                "tools": {}
            },
            "execution": { "blocked": null, "pending": null }
        });
        let ctx = Context::new(&doc, "call_1", "test");
        let plugin = PermissionPlugin;

        plugin.before_tool_execute(&ctx, "any_tool", &json!({})).await;

        // Should block - verify via has_changes
        assert!(ctx.has_changes());
    }

    #[tokio::test]
    async fn test_permission_plugin_ask() {
        let doc = json!({
            "permissions": {
                "default_behavior": "ask",
                "tools": {}
            },
            "execution": { "blocked": null, "pending": null }
        });
        let ctx = Context::new(&doc, "call_1", "test");
        let plugin = PermissionPlugin;

        plugin.before_tool_execute(&ctx, "test_tool", &json!({})).await;

        // Should create pending interaction - verify via has_changes
        assert!(ctx.has_changes());
    }
}
