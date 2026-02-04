//! Agent plugin system with Phase-based execution.
//!
//! Plugins extend agent behavior by responding to execution phases.
//! Each phase receives a mutable `TurnContext` for reading/writing state.
//!
//! # Phases
//!
//! - `SessionStart` / `SessionEnd` - Session lifecycle (called once)
//! - `TurnStart` / `TurnEnd` - Turn lifecycle
//! - `BeforeInference` / `AfterInference` - LLM call lifecycle
//! - `BeforeToolExecute` / `AfterToolExecute` - Tool execution lifecycle
//!
//! # Example
//!
//! ```ignore
//! use carve_agent::prelude::*;
//!
//! struct MyPlugin;
//!
//! #[async_trait]
//! impl AgentPlugin for MyPlugin {
//!     fn id(&self) -> &str { "my_plugin" }
//!
//!     async fn on_phase(&self, phase: Phase, turn: &mut TurnContext<'_>) {
//!         match phase {
//!             Phase::TurnStart => {
//!                 turn.system(format!("Time: {}", chrono::Local::now()));
//!             }
//!             Phase::BeforeInference => {
//!                 turn.exclude("dangerous_tool");
//!             }
//!             Phase::AfterToolExecute => {
//!                 if turn.tool_id() == Some("read_file") {
//!                     turn.reminder("Check for sensitive data.");
//!                 }
//!             }
//!             _ => {}
//!         }
//!     }
//! }
//! ```

use crate::phase::{Phase, TurnContext};
use async_trait::async_trait;
use serde_json::Value;

/// Plugin trait for extending agent behavior.
///
/// Plugins implement a single `on_phase` method that responds to all
/// execution phases. This provides a unified, simple interface for
/// extending the agent loop.
///
/// # Phase Handling
///
/// Use pattern matching to handle specific phases:
///
/// ```ignore
/// async fn on_phase(&self, phase: Phase, turn: &mut TurnContext<'_>) {
///     match phase {
///         Phase::SessionStart => { /* initialize */ }
///         Phase::TurnStart => { /* prepare context */ }
///         Phase::BeforeInference => { /* inject context, filter tools */ }
///         Phase::AfterInference => { /* process response */ }
///         Phase::BeforeToolExecute => { /* check permissions */ }
///         Phase::AfterToolExecute => { /* add reminders */ }
///         Phase::TurnEnd => { /* cleanup */ }
///         Phase::SessionEnd => { /* finalize */ }
///     }
/// }
/// ```
///
/// # Context Manipulation
///
/// Through `TurnContext`, plugins can:
///
/// - **Inject context**: `turn.system()`, `turn.session()`, `turn.reminder()`
/// - **Filter tools**: `turn.exclude()`, `turn.include_only()`
/// - **Control execution**: `turn.block()`, `turn.pending()`, `turn.confirm()`
/// - **Store data**: `turn.set()`, `turn.get()`
#[async_trait]
pub trait AgentPlugin: Send + Sync {
    /// Plugin identifier for logging and debugging.
    fn id(&self) -> &str;

    /// Respond to an execution phase.
    ///
    /// This is the single entry point for all plugin logic. Use pattern
    /// matching on `phase` to handle specific phases.
    ///
    /// # Arguments
    ///
    /// - `phase`: The current execution phase
    /// - `turn`: Mutable context for the current turn
    async fn on_phase(&self, phase: Phase, turn: &mut TurnContext<'_>);

    /// Provide initial data for this plugin.
    ///
    /// Returns a tuple of (key, initial_value) that will be stored
    /// in `TurnContext.data` at session start.
    ///
    /// # Example
    ///
    /// ```ignore
    /// fn initial_data(&self) -> Option<(&'static str, Value)> {
    ///     Some(("reminders", json!([])))
    /// }
    /// ```
    fn initial_data(&self) -> Option<(&'static str, Value)> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::phase::TurnContext;
    use crate::session::Session;
    use crate::state_types::Interaction;
    use crate::traits::tool::ToolDescriptor;
    use crate::types::ToolCall;
    use serde_json::json;

    // =========================================================================
    // Test Plugin Implementations
    // =========================================================================

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

        async fn on_phase(&self, phase: Phase, turn: &mut TurnContext<'_>) {
            match phase {
                Phase::TurnStart => {
                    turn.system("Test system context");
                }
                Phase::BeforeInference => {
                    turn.session("Test session context");
                }
                _ => {}
            }
        }

        fn initial_data(&self) -> Option<(&'static str, Value)> {
            Some(("test_data", json!({ "initialized": true })))
        }
    }

    struct NoOpPlugin;

    #[async_trait]
    impl AgentPlugin for NoOpPlugin {
        fn id(&self) -> &str {
            "noop"
        }

        async fn on_phase(&self, _phase: Phase, _turn: &mut TurnContext<'_>) {
            // No-op
        }
    }

    struct ContextInjectionPlugin;

    #[async_trait]
    impl AgentPlugin for ContextInjectionPlugin {
        fn id(&self) -> &str {
            "context_injection"
        }

        async fn on_phase(&self, phase: Phase, turn: &mut TurnContext<'_>) {
            match phase {
                Phase::TurnStart => {
                    turn.system("Current time: 2024-01-01");
                }
                Phase::BeforeInference => {
                    turn.session("Remember to be helpful.");
                    turn.exclude("dangerous_tool");
                }
                Phase::AfterToolExecute => {
                    if turn.tool_id() == Some("read_file") {
                        turn.reminder("Check for sensitive data.");
                    }
                }
                _ => {}
            }
        }
    }

    struct PermissionPlugin {
        denied_tools: Vec<String>,
    }

    impl PermissionPlugin {
        fn new(denied: Vec<&str>) -> Self {
            Self {
                denied_tools: denied.into_iter().map(String::from).collect(),
            }
        }
    }

    #[async_trait]
    impl AgentPlugin for PermissionPlugin {
        fn id(&self) -> &str {
            "permission"
        }

        async fn on_phase(&self, phase: Phase, turn: &mut TurnContext<'_>) {
            if phase != Phase::BeforeToolExecute {
                return;
            }

            if let Some(tool_id) = turn.tool_id() {
                if self.denied_tools.contains(&tool_id.to_string()) {
                    turn.block("Permission denied");
                }
            }
        }
    }

    struct ConfirmationPlugin {
        confirm_tools: Vec<String>,
    }

    impl ConfirmationPlugin {
        fn new(tools: Vec<&str>) -> Self {
            Self {
                confirm_tools: tools.into_iter().map(String::from).collect(),
            }
        }
    }

    #[async_trait]
    impl AgentPlugin for ConfirmationPlugin {
        fn id(&self) -> &str {
            "confirmation"
        }

        async fn on_phase(&self, phase: Phase, turn: &mut TurnContext<'_>) {
            if phase != Phase::BeforeToolExecute {
                return;
            }

            if let Some(tool_id) = turn.tool_id() {
                if self.confirm_tools.contains(&tool_id.to_string()) && !turn.tool_pending() {
                    turn.pending(
                        Interaction::new("confirm", "confirm")
                            .with_message("Execute this tool?"),
                    );
                }
            }
        }
    }

    // =========================================================================
    // Helper Functions
    // =========================================================================

    fn mock_session() -> Session {
        Session::new("test-session")
    }

    fn mock_tools() -> Vec<ToolDescriptor> {
        vec![
            ToolDescriptor::new("read_file", "Read File", "Read a file"),
            ToolDescriptor::new("write_file", "Write File", "Write a file"),
            ToolDescriptor::new("dangerous_tool", "Dangerous", "Dangerous operation"),
        ]
    }

    // =========================================================================
    // Tests
    // =========================================================================

    #[test]
    fn test_plugin_id() {
        let plugin = TestPlugin::new("test");
        assert_eq!(plugin.id(), "test");
    }

    #[test]
    fn test_plugin_initial_data() {
        let plugin = TestPlugin::new("test");
        let data = plugin.initial_data();

        assert!(data.is_some());
        let (key, value) = data.unwrap();
        assert_eq!(key, "test_data");
        assert_eq!(value["initialized"], true);
    }

    #[test]
    fn test_noop_plugin_no_initial_data() {
        let plugin = NoOpPlugin;
        assert!(plugin.initial_data().is_none());
    }

    #[tokio::test]
    async fn test_plugin_on_phase_turn_start() {
        let plugin = TestPlugin::new("test");
        let session = mock_session();
        let mut turn = TurnContext::new(&session, vec![]);

        plugin.on_phase(Phase::TurnStart, &mut turn).await;

        assert_eq!(turn.system_context.len(), 1);
        assert_eq!(turn.system_context[0], "Test system context");
    }

    #[tokio::test]
    async fn test_plugin_on_phase_before_inference() {
        let plugin = TestPlugin::new("test");
        let session = mock_session();
        let mut turn = TurnContext::new(&session, vec![]);

        plugin.on_phase(Phase::BeforeInference, &mut turn).await;

        assert_eq!(turn.session_context.len(), 1);
        assert_eq!(turn.session_context[0], "Test session context");
    }

    #[tokio::test]
    async fn test_context_injection_plugin() {
        let plugin = ContextInjectionPlugin;
        let session = mock_session();
        let tools = mock_tools();
        let mut turn = TurnContext::new(&session, tools);

        // TurnStart - adds system context
        plugin.on_phase(Phase::TurnStart, &mut turn).await;
        assert_eq!(turn.system_context[0], "Current time: 2024-01-01");

        // BeforeInference - adds session context and filters tools
        plugin.on_phase(Phase::BeforeInference, &mut turn).await;
        assert_eq!(turn.session_context[0], "Remember to be helpful.");
        assert!(!turn.tools.iter().any(|t| t.id == "dangerous_tool"));

        // AfterToolExecute - adds reminder for read_file
        let call = ToolCall::new("call_1", "read_file", json!({}));
        turn.tool = Some(crate::phase::ToolContext::new(&call));
        plugin.on_phase(Phase::AfterToolExecute, &mut turn).await;
        assert_eq!(turn.system_reminders[0], "Check for sensitive data.");
    }

    #[tokio::test]
    async fn test_permission_plugin_blocks_tool() {
        let plugin = PermissionPlugin::new(vec!["dangerous_tool"]);
        let session = mock_session();
        let mut turn = TurnContext::new(&session, vec![]);

        let call = ToolCall::new("call_1", "dangerous_tool", json!({}));
        turn.tool = Some(crate::phase::ToolContext::new(&call));

        plugin.on_phase(Phase::BeforeToolExecute, &mut turn).await;

        assert!(turn.tool_blocked());
    }

    #[tokio::test]
    async fn test_permission_plugin_allows_tool() {
        let plugin = PermissionPlugin::new(vec!["dangerous_tool"]);
        let session = mock_session();
        let mut turn = TurnContext::new(&session, vec![]);

        let call = ToolCall::new("call_1", "read_file", json!({}));
        turn.tool = Some(crate::phase::ToolContext::new(&call));

        plugin.on_phase(Phase::BeforeToolExecute, &mut turn).await;

        assert!(!turn.tool_blocked());
    }

    #[tokio::test]
    async fn test_confirmation_plugin_sets_pending() {
        let plugin = ConfirmationPlugin::new(vec!["write_file"]);
        let session = mock_session();
        let mut turn = TurnContext::new(&session, vec![]);

        let call = ToolCall::new("call_1", "write_file", json!({}));
        turn.tool = Some(crate::phase::ToolContext::new(&call));

        plugin.on_phase(Phase::BeforeToolExecute, &mut turn).await;

        assert!(turn.tool_pending());
    }

    #[tokio::test]
    async fn test_confirmation_plugin_skips_non_matching() {
        let plugin = ConfirmationPlugin::new(vec!["write_file"]);
        let session = mock_session();
        let mut turn = TurnContext::new(&session, vec![]);

        let call = ToolCall::new("call_1", "read_file", json!({}));
        turn.tool = Some(crate::phase::ToolContext::new(&call));

        plugin.on_phase(Phase::BeforeToolExecute, &mut turn).await;

        assert!(!turn.tool_pending());
    }

    #[tokio::test]
    async fn test_multiple_plugins_compose() {
        let plugins: Vec<Box<dyn AgentPlugin>> = vec![
            Box::new(ContextInjectionPlugin),
            Box::new(PermissionPlugin::new(vec!["dangerous_tool"])),
        ];

        let session = mock_session();
        let tools = mock_tools();
        let mut turn = TurnContext::new(&session, tools);

        // Run all plugins for TurnStart
        for plugin in &plugins {
            plugin.on_phase(Phase::TurnStart, &mut turn).await;
        }
        assert!(!turn.system_context.is_empty());

        // Run all plugins for BeforeInference
        for plugin in &plugins {
            plugin.on_phase(Phase::BeforeInference, &mut turn).await;
        }
        assert!(!turn.session_context.is_empty());
        assert!(!turn.tools.iter().any(|t| t.id == "dangerous_tool"));

        // Run all plugins for BeforeToolExecute with dangerous tool
        let call = ToolCall::new("call_1", "dangerous_tool", json!({}));
        turn.tool = Some(crate::phase::ToolContext::new(&call));
        for plugin in &plugins {
            plugin.on_phase(Phase::BeforeToolExecute, &mut turn).await;
        }
        assert!(turn.tool_blocked());
    }

    #[tokio::test]
    async fn test_noop_plugin_all_phases() {
        let plugin = NoOpPlugin;
        let session = mock_session();
        let mut turn = TurnContext::new(&session, vec![]);

        // All phases should be callable without panic or side effects
        plugin.on_phase(Phase::SessionStart, &mut turn).await;
        plugin.on_phase(Phase::TurnStart, &mut turn).await;
        plugin.on_phase(Phase::BeforeInference, &mut turn).await;
        plugin.on_phase(Phase::AfterInference, &mut turn).await;
        plugin.on_phase(Phase::BeforeToolExecute, &mut turn).await;
        plugin.on_phase(Phase::AfterToolExecute, &mut turn).await;
        plugin.on_phase(Phase::TurnEnd, &mut turn).await;
        plugin.on_phase(Phase::SessionEnd, &mut turn).await;

        // Context should be unchanged
        assert!(turn.system_context.is_empty());
        assert!(turn.session_context.is_empty());
        assert!(turn.system_reminders.is_empty());
    }
}
