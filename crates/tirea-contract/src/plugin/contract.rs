//! Agent plugin system with Phase-based execution.
//!
//! Plugins extend agent behavior by responding to execution phases.
//! Each phase receives its dedicated typed context.
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
//! use tirea::prelude::*;
//!
//! struct MyPlugin;
//!
//! #[async_trait]
//! impl AgentPlugin for MyPlugin {
//!     fn id(&self) -> &str { "my_plugin" }
//!
//!     async fn before_inference(&self, ctx: &mut BeforeInferenceContext<'_, '_>) {
//!         ctx.add_system_context(format!("Time: {}", chrono::Local::now()));
//!         ctx.exclude_tool("dangerous_tool");
//!     }
//!
//!     async fn after_tool_execute(&self, ctx: &mut AfterToolExecuteContext<'_, '_>) {
//!         ctx.add_system_reminder("Check for sensitive data.");
//!     }
//! }
//! ```

use crate::plugin::phase::{
    AfterInferenceContext, AfterToolExecuteContext, BeforeInferenceContext,
    BeforeToolExecuteContext, Phase, RunEndContext, RunStartContext, StepContext, StepEndContext,
    StepStartContext,
};
use async_trait::async_trait;

/// Plugin trait for extending agent behavior.
///
/// Plugins implement phase-specific methods (`before_inference`, `before_tool_execute`, ...)
/// for compile-time constrained access to phase capabilities.
///
#[async_trait]
pub trait AgentPlugin: Send + Sync {
    /// Plugin identifier for logging and debugging.
    fn id(&self) -> &str;

    async fn run_start(&self, ctx: &mut RunStartContext<'_, '_>) {
        #[allow(deprecated)]
        self.on_phase(Phase::RunStart, ctx.step_mut()).await;
    }

    async fn step_start(&self, ctx: &mut StepStartContext<'_, '_>) {
        #[allow(deprecated)]
        self.on_phase(Phase::StepStart, ctx.step_mut()).await;
    }

    async fn before_inference(&self, ctx: &mut BeforeInferenceContext<'_, '_>) {
        #[allow(deprecated)]
        self.on_phase(Phase::BeforeInference, ctx.step_mut()).await;
    }

    async fn after_inference(&self, ctx: &mut AfterInferenceContext<'_, '_>) {
        #[allow(deprecated)]
        self.on_phase(Phase::AfterInference, ctx.step_mut()).await;
    }

    async fn before_tool_execute(&self, ctx: &mut BeforeToolExecuteContext<'_, '_>) {
        #[allow(deprecated)]
        self.on_phase(Phase::BeforeToolExecute, ctx.step_mut())
            .await;
    }

    async fn after_tool_execute(&self, ctx: &mut AfterToolExecuteContext<'_, '_>) {
        #[allow(deprecated)]
        self.on_phase(Phase::AfterToolExecute, ctx.step_mut()).await;
    }

    async fn step_end(&self, ctx: &mut StepEndContext<'_, '_>) {
        #[allow(deprecated)]
        self.on_phase(Phase::StepEnd, ctx.step_mut()).await;
    }

    async fn run_end(&self, ctx: &mut RunEndContext<'_, '_>) {
        #[allow(deprecated)]
        self.on_phase(Phase::RunEnd, ctx.step_mut()).await;
    }

    #[deprecated(note = "implement phase-specific methods instead")]
    async fn on_phase(&self, _phase: Phase, _step: &mut StepContext<'_>) {}
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::Interaction;
    use crate::testing::TestFixture;
    use crate::thread::ToolCall;
    use crate::tool::contract::ToolDescriptor;
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

        async fn on_phase(&self, phase: Phase, step: &mut StepContext<'_>) {
            match phase {
                Phase::BeforeInference => {
                    step.system("Test system context");
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

        async fn on_phase(&self, _phase: Phase, _step: &mut StepContext<'_>) {
            // No-op
        }
    }

    struct ContextInjectionPlugin;

    #[async_trait]
    impl AgentPlugin for ContextInjectionPlugin {
        fn id(&self) -> &str {
            "context_injection"
        }

        async fn on_phase(&self, phase: Phase, step: &mut StepContext<'_>) {
            match phase {
                Phase::BeforeInference => {
                    step.system("Current time: 2024-01-01");
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

        async fn on_phase(&self, phase: Phase, step: &mut StepContext<'_>) {
            if phase != Phase::BeforeToolExecute {
                return;
            }

            if let Some(tool_id) = step.tool_name() {
                if self.denied_tools.contains(&tool_id.to_string()) {
                    step.deny("Permission denied");
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

        async fn on_phase(&self, phase: Phase, step: &mut StepContext<'_>) {
            if phase != Phase::BeforeToolExecute {
                return;
            }

            if let Some(tool_id) = step.tool_name() {
                if self.confirm_tools.contains(&tool_id.to_string()) && !step.tool_pending() {
                    step.ask(
                        Interaction::new("confirm", "confirm").with_message("Execute this tool?"),
                    );
                }
            }
        }
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

    #[tokio::test]
    async fn test_plugin_on_phase_step_start() {
        let plugin = TestPlugin::new("test");
        let fix = TestFixture::new();
        let mut step = fix.step(vec![]);

        plugin.on_phase(Phase::StepStart, &mut step).await;

        assert!(step.system_context.is_empty());
        assert!(step.session_context.is_empty());
    }

    #[tokio::test]
    async fn test_plugin_on_phase_before_inference() {
        let plugin = TestPlugin::new("test");
        let fix = TestFixture::new();
        let mut step = fix.step(vec![]);

        plugin.on_phase(Phase::BeforeInference, &mut step).await;

        assert_eq!(step.system_context.len(), 1);
        assert_eq!(step.system_context[0], "Test system context");
        assert_eq!(step.session_context.len(), 1);
        assert_eq!(step.session_context[0], "Test thread context");
    }

    #[tokio::test]
    async fn test_context_injection_plugin() {
        let plugin = ContextInjectionPlugin;
        let fix = TestFixture::new();
        let mut step = fix.step(mock_tools());

        // BeforeInference - adds prompt context and filters tools
        plugin.on_phase(Phase::BeforeInference, &mut step).await;
        assert_eq!(step.system_context[0], "Current time: 2024-01-01");
        assert_eq!(step.session_context[0], "Remember to be helpful.");
        assert!(!step.tools.iter().any(|t| t.id == "dangerous_tool"));

        // AfterToolExecute - adds reminder for read_file
        let call = ToolCall::new("call_1", "read_file", json!({}));
        step.tool = Some(crate::plugin::phase::ToolContext::new(&call));
        plugin.on_phase(Phase::AfterToolExecute, &mut step).await;
        assert_eq!(step.system_reminders[0], "Check for sensitive data.");
    }

    #[tokio::test]
    async fn test_permission_plugin_blocks_tool() {
        let plugin = PermissionPlugin::new(vec!["dangerous_tool"]);
        let fix = TestFixture::new();
        let mut step = fix.step(vec![]);

        let call = ToolCall::new("call_1", "dangerous_tool", json!({}));
        step.tool = Some(crate::plugin::phase::ToolContext::new(&call));

        plugin.on_phase(Phase::BeforeToolExecute, &mut step).await;

        assert!(step.tool_blocked());
    }

    #[tokio::test]
    async fn test_permission_plugin_allows_tool() {
        let plugin = PermissionPlugin::new(vec!["dangerous_tool"]);
        let fix = TestFixture::new();
        let mut step = fix.step(vec![]);

        let call = ToolCall::new("call_1", "read_file", json!({}));
        step.tool = Some(crate::plugin::phase::ToolContext::new(&call));

        plugin.on_phase(Phase::BeforeToolExecute, &mut step).await;

        assert!(!step.tool_blocked());
    }

    #[tokio::test]
    async fn test_confirmation_plugin_sets_pending() {
        let plugin = ConfirmationPlugin::new(vec!["write_file"]);
        let fix = TestFixture::new();
        let mut step = fix.step(vec![]);

        let call = ToolCall::new("call_1", "write_file", json!({}));
        step.tool = Some(crate::plugin::phase::ToolContext::new(&call));

        plugin.on_phase(Phase::BeforeToolExecute, &mut step).await;

        assert!(step.tool_pending());
    }

    #[tokio::test]
    async fn test_confirmation_plugin_skips_non_matching() {
        let plugin = ConfirmationPlugin::new(vec!["write_file"]);
        let fix = TestFixture::new();
        let mut step = fix.step(vec![]);

        let call = ToolCall::new("call_1", "read_file", json!({}));
        step.tool = Some(crate::plugin::phase::ToolContext::new(&call));

        plugin.on_phase(Phase::BeforeToolExecute, &mut step).await;

        assert!(!step.tool_pending());
    }

    #[tokio::test]
    async fn test_multiple_plugins_compose() {
        let plugins: Vec<Box<dyn AgentPlugin>> = vec![
            Box::new(ContextInjectionPlugin),
            Box::new(PermissionPlugin::new(vec!["dangerous_tool"])),
        ];

        let fix = TestFixture::new();
        let mut step = fix.step(mock_tools());

        // Run all plugins for StepStart
        for plugin in &plugins {
            plugin.on_phase(Phase::StepStart, &mut step).await;
        }
        assert!(step.system_context.is_empty());

        // Run all plugins for BeforeInference
        for plugin in &plugins {
            plugin.on_phase(Phase::BeforeInference, &mut step).await;
        }
        assert!(!step.system_context.is_empty());
        assert!(!step.session_context.is_empty());
        assert!(!step.tools.iter().any(|t| t.id == "dangerous_tool"));

        // Run all plugins for BeforeToolExecute with dangerous tool
        let call = ToolCall::new("call_1", "dangerous_tool", json!({}));
        step.tool = Some(crate::plugin::phase::ToolContext::new(&call));
        for plugin in &plugins {
            plugin.on_phase(Phase::BeforeToolExecute, &mut step).await;
        }
        assert!(step.tool_blocked());
    }

    #[tokio::test]
    async fn test_noop_plugin_all_phases() {
        let plugin = NoOpPlugin;
        let fix = TestFixture::new();
        let mut step = fix.step(vec![]);

        // All phases should be callable without panic or side effects
        plugin.on_phase(Phase::RunStart, &mut step).await;
        plugin.on_phase(Phase::StepStart, &mut step).await;
        plugin.on_phase(Phase::BeforeInference, &mut step).await;
        plugin.on_phase(Phase::AfterInference, &mut step).await;
        plugin.on_phase(Phase::BeforeToolExecute, &mut step).await;
        plugin.on_phase(Phase::AfterToolExecute, &mut step).await;
        plugin.on_phase(Phase::StepEnd, &mut step).await;
        plugin.on_phase(Phase::RunEnd, &mut step).await;

        // Context should be unchanged
        assert!(step.system_context.is_empty());
        assert!(step.session_context.is_empty());
        assert!(step.system_reminders.is_empty());
    }
}
