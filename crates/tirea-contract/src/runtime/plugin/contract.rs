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

use crate::runtime::plugin::phase::{
    AfterInferenceContext, AfterToolExecuteContext, BeforeInferenceContext,
    BeforeToolExecuteContext, RunEndContext, RunStartContext, StepEndContext, StepStartContext,
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

    async fn run_start(&self, _ctx: &mut RunStartContext<'_, '_>) {}

    async fn step_start(&self, _ctx: &mut StepStartContext<'_, '_>) {}

    async fn before_inference(&self, _ctx: &mut BeforeInferenceContext<'_, '_>) {}

    async fn after_inference(&self, _ctx: &mut AfterInferenceContext<'_, '_>) {}

    async fn before_tool_execute(&self, _ctx: &mut BeforeToolExecuteContext<'_, '_>) {}

    async fn after_tool_execute(&self, _ctx: &mut AfterToolExecuteContext<'_, '_>) {}

    async fn step_end(&self, _ctx: &mut StepEndContext<'_, '_>) {}

    async fn run_end(&self, _ctx: &mut RunEndContext<'_, '_>) {}
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::plugin::phase::{StepContext, SuspendTicket, ToolContext};
    use crate::runtime::tool_call::Suspension;
    use crate::runtime::{PendingToolCall, ToolCallResumeMode};
    use crate::testing::TestFixture;
    use crate::thread::ToolCall;
    use crate::runtime::tool_call::ToolDescriptor;
    use serde_json::json;

    async fn run_step_start(plugin: &dyn AgentPlugin, step: &mut StepContext<'_>) {
        let mut ctx = StepStartContext::new(step);
        plugin.step_start(&mut ctx).await;
    }

    async fn run_before_inference(plugin: &dyn AgentPlugin, step: &mut StepContext<'_>) {
        let mut ctx = BeforeInferenceContext::new(step);
        plugin.before_inference(&mut ctx).await;
    }

    async fn run_after_inference(plugin: &dyn AgentPlugin, step: &mut StepContext<'_>) {
        let mut ctx = AfterInferenceContext::new(step);
        plugin.after_inference(&mut ctx).await;
    }

    fn test_suspend_ticket(interaction: Suspension) -> SuspendTicket {
        let tool_name = interaction
            .action
            .strip_prefix("tool:")
            .unwrap_or("TestSuspend")
            .to_string();
        SuspendTicket::new(
            interaction.clone(),
            PendingToolCall::new(interaction.id, tool_name, interaction.parameters),
            ToolCallResumeMode::PassDecisionToTool,
        )
    }

    async fn run_before_tool_execute(plugin: &dyn AgentPlugin, step: &mut StepContext<'_>) {
        let mut ctx = BeforeToolExecuteContext::new(step);
        plugin.before_tool_execute(&mut ctx).await;
    }

    async fn run_after_tool_execute(plugin: &dyn AgentPlugin, step: &mut StepContext<'_>) {
        let mut ctx = AfterToolExecuteContext::new(step);
        plugin.after_tool_execute(&mut ctx).await;
    }

    async fn run_run_start(plugin: &dyn AgentPlugin, step: &mut StepContext<'_>) {
        let mut ctx = RunStartContext::new(step);
        plugin.run_start(&mut ctx).await;
    }

    async fn run_step_end(plugin: &dyn AgentPlugin, step: &mut StepContext<'_>) {
        let mut ctx = StepEndContext::new(step);
        plugin.step_end(&mut ctx).await;
    }

    async fn run_run_end(plugin: &dyn AgentPlugin, step: &mut StepContext<'_>) {
        let mut ctx = RunEndContext::new(step);
        plugin.run_end(&mut ctx).await;
    }

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

        async fn before_inference(&self, ctx: &mut BeforeInferenceContext<'_, '_>) {
            ctx.add_system_context("Test system context");
            ctx.add_session_message("Test thread context");
        }
    }

    struct NoOpPlugin;

    #[async_trait]
    impl AgentPlugin for NoOpPlugin {
        fn id(&self) -> &str {
            "noop"
        }
    }

    struct ContextInjectionPlugin;

    #[async_trait]
    impl AgentPlugin for ContextInjectionPlugin {
        fn id(&self) -> &str {
            "context_injection"
        }

        async fn before_inference(&self, ctx: &mut BeforeInferenceContext<'_, '_>) {
            ctx.add_system_context("Current time: 2024-01-01");
            ctx.add_session_message("Remember to be helpful.");
            ctx.exclude_tool("dangerous_tool");
        }

        async fn after_tool_execute(&self, ctx: &mut AfterToolExecuteContext<'_, '_>) {
            if ctx.tool_result().tool_name == "read_file" {
                ctx.add_system_reminder("Check for sensitive data.");
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

        async fn before_tool_execute(&self, ctx: &mut BeforeToolExecuteContext<'_, '_>) {
            if let Some(tool_id) = ctx.tool_name() {
                if self.denied_tools.contains(&tool_id.to_string()) {
                    ctx.block("Permission denied");
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

        async fn before_tool_execute(&self, ctx: &mut BeforeToolExecuteContext<'_, '_>) {
            if let Some(tool_id) = ctx.tool_name() {
                if self.confirm_tools.contains(&tool_id.to_string()) {
                    ctx.suspend(test_suspend_ticket(
                        Suspension::new("confirm", "confirm").with_message("Execute this tool?"),
                    ));
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
    async fn test_plugin_step_start_hook() {
        let plugin = TestPlugin::new("test");
        let fix = TestFixture::new();
        let mut step = fix.step(vec![]);

        run_step_start(&plugin, &mut step).await;

        assert!(step.system_context.is_empty());
        assert!(step.session_context.is_empty());
    }

    #[tokio::test]
    async fn test_plugin_before_inference_hook() {
        let plugin = TestPlugin::new("test");
        let fix = TestFixture::new();
        let mut step = fix.step(vec![]);

        run_before_inference(&plugin, &mut step).await;

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
        run_before_inference(&plugin, &mut step).await;
        assert_eq!(step.system_context[0], "Current time: 2024-01-01");
        assert_eq!(step.session_context[0], "Remember to be helpful.");
        assert!(!step.tools.iter().any(|t| t.id == "dangerous_tool"));

        // AfterToolExecute - adds reminder for read_file
        let call = ToolCall::new("call_1", "read_file", json!({}));
        step.tool = Some(ToolContext::new(&call));
        step.set_tool_result(crate::runtime::tool_call::ToolResult::success(
            "read_file",
            json!({"ok": true}),
        ));
        run_after_tool_execute(&plugin, &mut step).await;
        assert_eq!(step.system_reminders[0], "Check for sensitive data.");
    }

    #[tokio::test]
    async fn test_permission_plugin_blocks_tool() {
        let plugin = PermissionPlugin::new(vec!["dangerous_tool"]);
        let fix = TestFixture::new();
        let mut step = fix.step(vec![]);

        let call = ToolCall::new("call_1", "dangerous_tool", json!({}));
        step.tool = Some(ToolContext::new(&call));

        run_before_tool_execute(&plugin, &mut step).await;

        assert!(step.tool_blocked());
    }

    #[tokio::test]
    async fn test_permission_plugin_allows_tool() {
        let plugin = PermissionPlugin::new(vec!["dangerous_tool"]);
        let fix = TestFixture::new();
        let mut step = fix.step(vec![]);

        let call = ToolCall::new("call_1", "read_file", json!({}));
        step.tool = Some(ToolContext::new(&call));

        run_before_tool_execute(&plugin, &mut step).await;

        assert!(!step.tool_blocked());
    }

    #[tokio::test]
    async fn test_confirmation_plugin_sets_pending() {
        let plugin = ConfirmationPlugin::new(vec!["write_file"]);
        let fix = TestFixture::new();
        let mut step = fix.step(vec![]);

        let call = ToolCall::new("call_1", "write_file", json!({}));
        step.tool = Some(ToolContext::new(&call));

        run_before_tool_execute(&plugin, &mut step).await;

        assert!(step.tool_pending());
    }

    #[tokio::test]
    async fn test_confirmation_plugin_skips_non_matching() {
        let plugin = ConfirmationPlugin::new(vec!["write_file"]);
        let fix = TestFixture::new();
        let mut step = fix.step(vec![]);

        let call = ToolCall::new("call_1", "read_file", json!({}));
        step.tool = Some(ToolContext::new(&call));

        run_before_tool_execute(&plugin, &mut step).await;

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
            run_step_start(plugin.as_ref(), &mut step).await;
        }
        assert!(step.system_context.is_empty());

        // Run all plugins for BeforeInference
        for plugin in &plugins {
            run_before_inference(plugin.as_ref(), &mut step).await;
        }
        assert!(!step.system_context.is_empty());
        assert!(!step.session_context.is_empty());
        assert!(!step.tools.iter().any(|t| t.id == "dangerous_tool"));

        // Run all plugins for BeforeToolExecute with dangerous tool
        let call = ToolCall::new("call_1", "dangerous_tool", json!({}));
        step.tool = Some(ToolContext::new(&call));
        for plugin in &plugins {
            run_before_tool_execute(plugin.as_ref(), &mut step).await;
        }
        assert!(step.tool_blocked());
    }

    #[tokio::test]
    async fn test_noop_plugin_all_phases() {
        let plugin = NoOpPlugin;
        let fix = TestFixture::new();
        let mut step = fix.step(vec![]);

        // All phases should be callable without panic or side effects
        run_run_start(&plugin, &mut step).await;
        run_step_start(&plugin, &mut step).await;
        run_before_inference(&plugin, &mut step).await;
        run_after_inference(&plugin, &mut step).await;
        run_before_tool_execute(&plugin, &mut step).await;
        run_after_tool_execute(&plugin, &mut step).await;
        run_step_end(&plugin, &mut step).await;
        run_run_end(&plugin, &mut step).await;

        // Context should be unchanged
        assert!(step.system_context.is_empty());
        assert!(step.session_context.is_empty());
        assert!(step.system_reminders.is_empty());
    }
}
