//! Agent plugin system.
//!
//! Plugins extend agent behavior through lifecycle hooks. Each hook receives
//! a Context for reading/writing state. Plugins can:
//! - Block tool execution via `ctx.block()`
//! - Request user interaction via `ctx.pending()`
//! - Modify permissions, messages, reminders, and context data
//!
//! # Example
//!
//! ```ignore
//! use carve_agent::prelude::*;
//!
//! struct LoggingPlugin;
//!
//! #[async_trait]
//! impl AgentPlugin for LoggingPlugin {
//!     fn id(&self) -> &str { "logging" }
//!
//!     async fn before_tool_execute(&self, ctx: &Context<'_>, tool_id: &str, _args: &Value) {
//!         ctx.add_reminder(format!("Executing: {}", tool_id));
//!     }
//!
//!     async fn after_tool_execute(&self, ctx: &Context<'_>, tool_id: &str, result: &ToolResult) {
//!         ctx.add_reminder(format!("Completed: {} ({:?})", tool_id, result.status));
//!     }
//! }
//! ```

use crate::traits::tool::ToolResult;
use async_trait::async_trait;
use carve_state::Context;
use serde_json::Value;

/// Plugin trait for extending agent behavior.
///
/// Plugins implement lifecycle hooks that are called at various points
/// during agent execution. All hooks have default no-op implementations,
/// so plugins only need to override the hooks they care about.
///
/// # Lifecycle Hooks
///
/// 1. `on_session_start` - Called once when the session begins
/// 2. `before_llm_request` - Called before each LLM API call
/// 3. `before_tool_execute` - Called before each tool execution
/// 4. `after_tool_execute` - Called after each tool execution
/// 5. `on_session_end` - Called once when the session ends
///
/// # State Management
///
/// Plugins modify state through the Context's extension traits:
/// - `ctx.block(reason)` - Block execution
/// - `ctx.pending(interaction)` - Request user interaction
/// - `ctx.allow_tool(id)` / `ctx.deny_tool(id)` - Modify permissions
/// - `ctx.add_reminder(text)` - Add reminders
#[async_trait]
pub trait AgentPlugin: Send + Sync {
    /// Plugin identifier for logging and debugging.
    fn id(&self) -> &str;

    /// Called when the session starts.
    ///
    /// Use this for initialization, setting up initial state, etc.
    async fn on_session_start(&self, _ctx: &Context<'_>) {}

    /// Called before each LLM request.
    ///
    /// Use this to inject context, modify messages, add reminders, etc.
    async fn before_llm_request(&self, _ctx: &Context<'_>) {}

    /// Called before tool execution.
    ///
    /// Use this to:
    /// - Check permissions and block execution if denied
    /// - Request user confirmation for sensitive operations
    /// - Log tool invocations
    ///
    /// To block execution, call `ctx.block(reason)`.
    /// To request user interaction, call `ctx.pending(interaction)`.
    async fn before_tool_execute(&self, _ctx: &Context<'_>, _tool_id: &str, _args: &Value) {}

    /// Called after tool execution.
    ///
    /// Use this to:
    /// - Log results
    /// - Update state based on results
    /// - Trigger follow-up actions
    async fn after_tool_execute(&self, _ctx: &Context<'_>, _tool_id: &str, _result: &ToolResult) {}

    /// Called when the session ends.
    ///
    /// Use this for cleanup, final logging, etc.
    async fn on_session_end(&self, _ctx: &Context<'_>) {}

    /// Provide initial state for this plugin.
    ///
    /// Returns a tuple of (state_path, initial_value) that will be merged
    /// into the session state when the plugin is registered.
    ///
    /// # Example
    ///
    /// ```ignore
    /// fn initial_state(&self) -> Option<(&'static str, Value)> {
    ///     Some(("permissions", json!({
    ///         "default_behavior": "ask",
    ///         "tools": {}
    ///     })))
    /// }
    /// ```
    fn initial_state(&self) -> Option<(&'static str, Value)> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    struct TestPlugin {
        id: String,
    }

    impl TestPlugin {
        fn new(id: impl Into<String>) -> Self {
            Self { id: id.into() }
        }
    }

    #[async_trait]
    impl AgentPlugin for TestPlugin {
        fn id(&self) -> &str {
            &self.id
        }

        fn initial_state(&self) -> Option<(&'static str, Value)> {
            Some(("test_plugin", json!({ "initialized": true })))
        }
    }

    #[test]
    fn test_plugin_id() {
        let plugin = TestPlugin::new("test");
        assert_eq!(plugin.id(), "test");
    }

    #[test]
    fn test_plugin_initial_state() {
        let plugin = TestPlugin::new("test");
        let state = plugin.initial_state();

        assert!(state.is_some());
        let (path, value) = state.unwrap();
        assert_eq!(path, "test_plugin");
        assert_eq!(value["initialized"], true);
    }

    struct NoOpPlugin;

    #[async_trait]
    impl AgentPlugin for NoOpPlugin {
        fn id(&self) -> &str {
            "noop"
        }
    }

    #[test]
    fn test_noop_plugin_no_initial_state() {
        let plugin = NoOpPlugin;
        assert!(plugin.initial_state().is_none());
    }

    #[tokio::test]
    async fn test_plugin_hooks_are_callable() {
        let plugin = NoOpPlugin;
        let doc = json!({});
        let ctx = Context::new(&doc, "call_1", "test");

        // All hooks should be callable without panic
        plugin.on_session_start(&ctx).await;
        plugin.before_llm_request(&ctx).await;
        plugin.before_tool_execute(&ctx, "test_tool", &json!({})).await;
        plugin
            .after_tool_execute(&ctx, "test_tool", &ToolResult::success("test", json!({})))
            .await;
        plugin.on_session_end(&ctx).await;
    }
}
