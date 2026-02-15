//! Frontend tool strategy plugin.
//!
//! Intercepts frontend tool execution and emits pending interaction intents.

use super::set_pending_and_push_intent;
use crate::phase::{Phase, StepContext};
use crate::plugin::AgentPlugin;
use crate::state_types::Interaction;
use async_trait::async_trait;
use carve_state::Context;
use std::collections::HashSet;

/// Strategy plugin that marks frontend tools as pending interactions.
///
/// When a tool call targets a frontend tool (defined with `execute: "frontend"`),
/// this plugin intercepts the execution in `BeforeToolExecute` and emits
/// a pending interaction intent. The interaction mechanism plugin consumes
/// those intents and applies the runtime gate state.
///
/// # Example
///
/// ```ignore
/// let frontend_tools: HashSet<String> = request.frontend_tools()
///     .iter()
///     .map(|t| t.name.clone())
///     .collect();
///
/// let plugin = FrontendToolPlugin::new(frontend_tools);
/// let config = AgentConfig::new("gpt-4").with_plugin(Arc::new(plugin));
/// ```
pub(crate) struct FrontendToolPlugin {
    /// Names of tools that should be executed on the frontend.
    pub(crate) frontend_tools: HashSet<String>,
}

impl FrontendToolPlugin {
    /// Create a new frontend tool plugin.
    ///
    /// # Arguments
    ///
    /// * `frontend_tools` - Set of tool names that should be executed on the frontend
    pub(crate) fn new(frontend_tools: HashSet<String>) -> Self {
        Self { frontend_tools }
    }

    /// Check if a tool should be executed on the frontend.
    pub(crate) fn is_frontend_tool(&self, name: &str) -> bool {
        self.frontend_tools.contains(name)
    }
}

#[async_trait]
impl AgentPlugin for FrontendToolPlugin {
    fn id(&self) -> &str {
        "frontend_tool"
    }

    async fn on_phase(&self, phase: Phase, step: &mut StepContext<'_>, _ctx: &Context<'_>) {
        if phase != Phase::BeforeToolExecute {
            return;
        }

        // Get tool info
        let Some(tool) = step.tool.as_ref() else {
            return;
        };

        // Check if this is a frontend tool
        if !self.is_frontend_tool(&tool.name) {
            return;
        }

        // Don't emit pending if tool is already blocked.
        if step.tool_blocked() {
            return;
        }

        // Create interaction for frontend execution
        // The tool call ID and arguments are passed to the client
        let interaction = Interaction::new(&tool.id, format!("tool:{}", tool.name))
            .with_parameters(tool.args.clone());

        set_pending_and_push_intent(step, interaction);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::phase::ToolContext;
    use crate::thread::Thread;
    use crate::types::ToolCall;
    use serde_json::json;

    #[test]
    fn identifies_frontend_tool_names() {
        let plugin = FrontendToolPlugin::new(["copy".to_string()].into_iter().collect());
        assert!(plugin.is_frontend_tool("copy"));
        assert!(!plugin.is_frontend_tool("search"));
    }

    #[tokio::test]
    async fn marks_frontend_tool_as_pending() {
        let plugin = FrontendToolPlugin::new(["copy".to_string()].into_iter().collect());
        let state = json!({});
        let ctx = Context::new(&state, "test", "test");
        let thread = Thread::new("t1");
        let mut step = StepContext::new(&thread, vec![]);
        let call = ToolCall::new("call_1", "copy", json!({"text":"hello"}));
        step.tool = Some(ToolContext::new(&call));

        plugin
            .on_phase(Phase::BeforeToolExecute, &mut step, &ctx)
            .await;

        let interaction = step
            .tool
            .as_ref()
            .and_then(|t| t.pending_interaction.as_ref())
            .expect("pending interaction should exist");
        assert_eq!(interaction.id, "call_1");
        assert_eq!(interaction.action, "tool:copy");
    }
}
