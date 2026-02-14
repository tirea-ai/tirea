//! Frontend tool strategy plugin.
//!
//! Intercepts frontend tool execution and emits pending interaction intents.

use super::push_pending_intent;
use crate::phase::{Phase, StepContext};
use crate::plugin::AgentPlugin;
use crate::state_types::Interaction;
use async_trait::async_trait;
use carve_state::Context;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

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

    /// Whether any frontend tools are configured.
    pub(crate) fn has_frontend_tools(&self) -> bool {
        !self.frontend_tools.is_empty()
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

        push_pending_intent(step, interaction);
    }
}

/// Stub `Tool` implementation for frontend-defined tools.
///
/// This provides a `ToolDescriptor` so the LLM knows the tool exists, but execution
/// is intercepted by `FrontendToolPlugin` before `execute` is ever called.
pub(crate) struct FrontendToolStub {
    /// Tool descriptor for the frontend tool stub.
    pub(crate) descriptor: crate::traits::tool::ToolDescriptor,
}

impl FrontendToolStub {
    /// Create from a frontend tool spec.
    pub(crate) fn from_spec(spec: &FrontendToolSpec) -> Self {
        let parameters = spec
            .parameters
            .clone()
            .unwrap_or_else(|| json!({"type": "object", "properties": {}}));
        Self {
            descriptor: crate::traits::tool::ToolDescriptor::new(
                &spec.name,
                &spec.name,
                &spec.description,
            )
            .with_parameters(parameters),
        }
    }
}

/// Protocol-agnostic frontend tool declaration used to install stub tools.
#[derive(Debug, Clone)]
pub(crate) struct FrontendToolSpec {
    /// Tool name.
    pub(crate) name: String,
    /// Tool description.
    pub(crate) description: String,
    /// Optional JSON schema for parameters.
    pub(crate) parameters: Option<Value>,
}

#[async_trait]
impl crate::traits::tool::Tool for FrontendToolStub {
    fn descriptor(&self) -> crate::traits::tool::ToolDescriptor {
        self.descriptor.clone()
    }

    async fn execute(
        &self,
        _args: Value,
        _ctx: &carve_state::Context<'_>,
    ) -> Result<crate::traits::tool::ToolResult, crate::traits::tool::ToolError> {
        // Should never be reached – FrontendToolPlugin intercepts before execution.
        Err(crate::traits::tool::ToolError::Internal(
            "frontend tool stub should not be executed directly".into(),
        ))
    }
}

/// Convert frontend tool definitions from an AG-UI request into `Arc<dyn Tool>` stubs
/// and merge them into the provided tools map.
pub(crate) fn merge_frontend_tools(
    tools: &mut HashMap<String, Arc<dyn crate::traits::tool::Tool>>,
    frontend_tools: &[FrontendToolSpec],
) {
    for spec in frontend_tools {
        tools
            .entry(spec.name.clone())
            .or_insert_with(|| Arc::new(FrontendToolStub::from_spec(spec)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn merge_frontend_tools_inserts_stub_tools() {
        let mut tools: HashMap<String, Arc<dyn crate::traits::tool::Tool>> = HashMap::new();
        let specs = vec![FrontendToolSpec {
            name: "copy_to_clipboard".to_string(),
            description: "Copy content".to_string(),
            parameters: Some(json!({
                "type": "object",
                "properties": { "text": { "type": "string" } }
            })),
        }];

        merge_frontend_tools(&mut tools, &specs);

        let tool = tools.get("copy_to_clipboard").expect("tool should exist");
        let descriptor = tool.descriptor();
        assert_eq!(descriptor.name, "copy_to_clipboard");
        assert_eq!(descriptor.description, "Copy content");
    }

    #[test]
    fn merge_frontend_tools_does_not_override_existing_tool() {
        let mut tools: HashMap<String, Arc<dyn crate::traits::tool::Tool>> = HashMap::new();
        let existing = FrontendToolStub::from_spec(&FrontendToolSpec {
            name: "copy_to_clipboard".to_string(),
            description: "Existing".to_string(),
            parameters: None,
        });
        tools.insert("copy_to_clipboard".to_string(), Arc::new(existing));

        let specs = vec![FrontendToolSpec {
            name: "copy_to_clipboard".to_string(),
            description: "New".to_string(),
            parameters: None,
        }];
        merge_frontend_tools(&mut tools, &specs);

        let descriptor = tools
            .get("copy_to_clipboard")
            .expect("tool should exist")
            .descriptor();
        assert_eq!(descriptor.description, "Existing");
    }
}
