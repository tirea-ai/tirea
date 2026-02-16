//! Agent plugin system with Phase-based execution.
//!
//! Plugins extend agent behavior by responding to execution phases.
//! Each phase receives a mutable `StepContext` for reading/writing state.
//!
//! # Phases
//!
//! - `RunStart` / `RunEnd` - Thread lifecycle (called once)
//! - `StepStart` / `StepEnd` - Step lifecycle
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
//!     async fn on_phase(&self, phase: Phase, step: &mut StepContext<'_>, ctx: &AgentState<'_>) {
//!         match phase {
//!             Phase::StepStart => {
//!                 step.system(format!("Time: {}", chrono::Local::now()));
//!             }
//!             Phase::BeforeInference => {
//!                 step.exclude("dangerous_tool");
//!             }
//!             Phase::AfterToolExecute => {
//!                 if step.tool_name() == Some("read_file") {
//!                     step.reminder("Check for sensitive data.");
//!                 }
//!             }
//!             _ => {}
//!         }
//!     }
//! }
//! ```

use crate::extension::phase::{Phase, StepContext};
use async_trait::async_trait;
use crate::AgentState;

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
/// async fn on_phase(&self, phase: Phase, step: &mut StepContext<'_>, ctx: &AgentState<'_>) {
///     match phase {
///         Phase::RunStart => { /* initialize */ }
///         Phase::StepStart => { /* prepare context */ }
///         Phase::BeforeInference => { /* inject context, filter tools */ }
///         Phase::AfterInference => { /* process response */ }
///         Phase::BeforeToolExecute => { /* check permissions */ }
///         Phase::AfterToolExecute => { /* add reminders */ }
///         Phase::StepEnd => { /* cleanup */ }
///         Phase::RunEnd => { /* finalize */ }
///     }
/// }
/// ```
///
/// # Context Manipulation
///
/// Through `StepContext`, plugins can:
///
/// - **Inject context**: `step.system()`, `step.session()`, `step.reminder()`
/// - **Filter tools**: `step.exclude()`, `step.include_only()`
/// - **Control execution**: `step.block()`, `step.pending()`, `step.confirm()`
/// - **Mutate state**: enqueue `step.pending_patches` or write through `ctx.state(...)`
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
    /// - `step`: Mutable context for the current step
    /// - `ctx`: AgentState for typed state access (read/write ops auto-collected)
    async fn on_phase(&self, phase: Phase, step: &mut StepContext<'_>, ctx: &AgentState<'_>);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extension::phase::StepContext;
    use crate::extension::state_types::Interaction;
    use crate::extension::traits::tool::ToolDescriptor;
    use carve_thread_model::{Thread, ToolCall};
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

        async fn on_phase(&self, phase: Phase, step: &mut StepContext<'_>, _ctx: &AgentState<'_>) {
            match phase {
                Phase::StepStart => {
                    step.system("Test system context");
                }
                Phase::BeforeInference => {
                    step.thread("Test thread context");
                }
                _ => {}
            }
        }
    }

    struct NoOpPlugin;

    #[async_trait]
    impl AgentPlugin for NoOpPlugin {
        fn id(&self) -> &str {
            "noop"
        }

        async fn on_phase(&self, _phase: Phase, _step: &mut StepContext<'_>, _ctx: &AgentState<'_>) {
            // No-op
        }
    }

    struct ContextInjectionPlugin;

    #[async_trait]
    impl AgentPlugin for ContextInjectionPlugin {
        fn id(&self) -> &str {
            "context_injection"
        }

        async fn on_phase(&self, phase: Phase, step: &mut StepContext<'_>, _ctx: &AgentState<'_>) {
            match phase {
                Phase::StepStart => {
                    step.system("Current time: 2024-01-01");
                }
                Phase::BeforeInference => {
                    step.thread("Remember to be helpful.");
                    step.exclude("dangerous_tool");
                }
                Phase::AfterToolExecute => {
                    if step.tool_name() == Some("read_file") {
                        step.reminder("Check for sensitive data.");
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

        async fn on_phase(&self, phase: Phase, step: &mut StepContext<'_>, _ctx: &AgentState<'_>) {
            if phase != Phase::BeforeToolExecute {
                return;
            }

            if let Some(tool_id) = step.tool_name() {
                if self.denied_tools.contains(&tool_id.to_string()) {
                    step.block("Permission denied");
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

        async fn on_phase(&self, phase: Phase, step: &mut StepContext<'_>, _ctx: &AgentState<'_>) {
            if phase != Phase::BeforeToolExecute {
                return;
            }

            if let Some(tool_id) = step.tool_name() {
                if self.confirm_tools.contains(&tool_id.to_string()) && !step.tool_pending() {
                    step.pending(
                        Interaction::new("confirm", "confirm").with_message("Execute this tool?"),
                    );
                }
            }
        }
    }

    // =========================================================================
    // Helper Functions
    // =========================================================================

    fn mock_thread() -> Thread {
        Thread::new("test-thread")
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

    fn test_ctx(doc: &serde_json::Value) -> AgentState<'_> {
        AgentState::new(doc, "test", "test")
    }

    #[tokio::test]
    async fn test_plugin_on_phase_step_start() {
        let plugin = TestPlugin::new("test");
        let thread = mock_thread();
        let mut step = StepContext::new(&thread, vec![]);
        let doc = json!({});
        let ctx = test_ctx(&doc);

        plugin.on_phase(Phase::StepStart, &mut step, &ctx).await;

        assert_eq!(step.system_context.len(), 1);
        assert_eq!(step.system_context[0], "Test system context");
    }

    #[tokio::test]
    async fn test_plugin_on_phase_before_inference() {
        let plugin = TestPlugin::new("test");
        let thread = mock_thread();
        let mut step = StepContext::new(&thread, vec![]);
        let doc = json!({});
        let ctx = test_ctx(&doc);

        plugin
            .on_phase(Phase::BeforeInference, &mut step, &ctx)
            .await;

        assert_eq!(step.session_context.len(), 1);
        assert_eq!(step.session_context[0], "Test thread context");
    }

    #[tokio::test]
    async fn test_context_injection_plugin() {
        let plugin = ContextInjectionPlugin;
        let thread = mock_thread();
        let tools = mock_tools();
        let mut step = StepContext::new(&thread, tools);
        let doc = json!({});
        let ctx = test_ctx(&doc);

        // StepStart - adds system context
        plugin.on_phase(Phase::StepStart, &mut step, &ctx).await;
        assert_eq!(step.system_context[0], "Current time: 2024-01-01");

        // BeforeInference - adds session context and filters tools
        plugin
            .on_phase(Phase::BeforeInference, &mut step, &ctx)
            .await;
        assert_eq!(step.session_context[0], "Remember to be helpful.");
        assert!(!step.tools.iter().any(|t| t.id == "dangerous_tool"));

        // AfterToolExecute - adds reminder for read_file
        let call = ToolCall::new("call_1", "read_file", json!({}));
        step.tool = Some(crate::extension::phase::ToolContext::new(&call));
        plugin
            .on_phase(Phase::AfterToolExecute, &mut step, &ctx)
            .await;
        assert_eq!(step.system_reminders[0], "Check for sensitive data.");
    }

    #[tokio::test]
    async fn test_permission_plugin_blocks_tool() {
        let plugin = PermissionPlugin::new(vec!["dangerous_tool"]);
        let thread = mock_thread();
        let mut step = StepContext::new(&thread, vec![]);
        let doc = json!({});
        let ctx = test_ctx(&doc);

        let call = ToolCall::new("call_1", "dangerous_tool", json!({}));
        step.tool = Some(crate::extension::phase::ToolContext::new(&call));

        plugin
            .on_phase(Phase::BeforeToolExecute, &mut step, &ctx)
            .await;

        assert!(step.tool_blocked());
    }

    #[tokio::test]
    async fn test_permission_plugin_allows_tool() {
        let plugin = PermissionPlugin::new(vec!["dangerous_tool"]);
        let thread = mock_thread();
        let mut step = StepContext::new(&thread, vec![]);
        let doc = json!({});
        let ctx = test_ctx(&doc);

        let call = ToolCall::new("call_1", "read_file", json!({}));
        step.tool = Some(crate::extension::phase::ToolContext::new(&call));

        plugin
            .on_phase(Phase::BeforeToolExecute, &mut step, &ctx)
            .await;

        assert!(!step.tool_blocked());
    }

    #[tokio::test]
    async fn test_confirmation_plugin_sets_pending() {
        let plugin = ConfirmationPlugin::new(vec!["write_file"]);
        let thread = mock_thread();
        let mut step = StepContext::new(&thread, vec![]);
        let doc = json!({});
        let ctx = test_ctx(&doc);

        let call = ToolCall::new("call_1", "write_file", json!({}));
        step.tool = Some(crate::extension::phase::ToolContext::new(&call));

        plugin
            .on_phase(Phase::BeforeToolExecute, &mut step, &ctx)
            .await;

        assert!(step.tool_pending());
    }

    #[tokio::test]
    async fn test_confirmation_plugin_skips_non_matching() {
        let plugin = ConfirmationPlugin::new(vec!["write_file"]);
        let thread = mock_thread();
        let mut step = StepContext::new(&thread, vec![]);
        let doc = json!({});
        let ctx = test_ctx(&doc);

        let call = ToolCall::new("call_1", "read_file", json!({}));
        step.tool = Some(crate::extension::phase::ToolContext::new(&call));

        plugin
            .on_phase(Phase::BeforeToolExecute, &mut step, &ctx)
            .await;

        assert!(!step.tool_pending());
    }

    #[tokio::test]
    async fn test_multiple_plugins_compose() {
        let plugins: Vec<Box<dyn AgentPlugin>> = vec![
            Box::new(ContextInjectionPlugin),
            Box::new(PermissionPlugin::new(vec!["dangerous_tool"])),
        ];

        let thread = mock_thread();
        let tools = mock_tools();
        let mut step = StepContext::new(&thread, tools);
        let doc = json!({});
        let ctx = test_ctx(&doc);

        // Run all plugins for StepStart
        for plugin in &plugins {
            plugin.on_phase(Phase::StepStart, &mut step, &ctx).await;
        }
        assert!(!step.system_context.is_empty());

        // Run all plugins for BeforeInference
        for plugin in &plugins {
            plugin
                .on_phase(Phase::BeforeInference, &mut step, &ctx)
                .await;
        }
        assert!(!step.session_context.is_empty());
        assert!(!step.tools.iter().any(|t| t.id == "dangerous_tool"));

        // Run all plugins for BeforeToolExecute with dangerous tool
        let call = ToolCall::new("call_1", "dangerous_tool", json!({}));
        step.tool = Some(crate::extension::phase::ToolContext::new(&call));
        for plugin in &plugins {
            plugin
                .on_phase(Phase::BeforeToolExecute, &mut step, &ctx)
                .await;
        }
        assert!(step.tool_blocked());
    }

    #[tokio::test]
    async fn test_noop_plugin_all_phases() {
        let plugin = NoOpPlugin;
        let thread = mock_thread();
        let mut step = StepContext::new(&thread, vec![]);
        let doc = json!({});
        let ctx = test_ctx(&doc);

        // All phases should be callable without panic or side effects
        plugin.on_phase(Phase::RunStart, &mut step, &ctx).await;
        plugin.on_phase(Phase::StepStart, &mut step, &ctx).await;
        plugin
            .on_phase(Phase::BeforeInference, &mut step, &ctx)
            .await;
        plugin
            .on_phase(Phase::AfterInference, &mut step, &ctx)
            .await;
        plugin
            .on_phase(Phase::BeforeToolExecute, &mut step, &ctx)
            .await;
        plugin
            .on_phase(Phase::AfterToolExecute, &mut step, &ctx)
            .await;
        plugin.on_phase(Phase::StepEnd, &mut step, &ctx).await;
        plugin.on_phase(Phase::RunEnd, &mut step, &ctx).await;

        // Context should be unchanged
        assert!(step.system_context.is_empty());
        assert!(step.session_context.is_empty());
        assert!(step.system_reminders.is_empty());
    }
}
