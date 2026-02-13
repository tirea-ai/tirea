//! AG-UI Frontend Tool Plugin.
//!
//! Intercepts frontend tool execution and creates pending interactions
//! for client-side handling.

use super::{AGUIToolDef, RunAgentRequest};
use crate::phase::{Phase, StepContext};
use crate::plugin::AgentPlugin;
use crate::state_types::Interaction;
use async_trait::async_trait;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

/// Plugin that handles frontend tool execution for AG-UI protocol.
///
/// When a tool call targets a frontend tool (defined with `execute: "frontend"`),
/// this plugin intercepts the execution in `BeforeToolExecute` and creates
/// a pending interaction. This causes the agent loop to emit `AgentEvent::Pending`,
/// which gets converted to AG-UI tool call events for client-side execution.
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

    /// Create from a RunAgentRequest.
    pub(crate) fn from_request(request: &RunAgentRequest) -> Self {
        let frontend_tools = request
            .frontend_tools()
            .iter()
            .map(|t| t.name.clone())
            .collect();
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

    async fn on_phase(&self, phase: Phase, step: &mut StepContext<'_>) {
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

        // Don't create pending if tool is already blocked (e.g., by PermissionPlugin)
        if step.tool_blocked() {
            return;
        }

        // Create interaction for frontend execution
        // The tool call ID and arguments are passed to the client
        let interaction = Interaction::new(&tool.id, format!("tool:{}", tool.name))
            .with_parameters(tool.args.clone());

        step.pending(interaction);
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
    /// Create from an AG-UI tool definition.
    pub(crate) fn from_agui_def(def: &AGUIToolDef) -> Self {
        let parameters = def
            .parameters
            .clone()
            .unwrap_or_else(|| json!({"type": "object", "properties": {}}));
        Self {
            descriptor: crate::traits::tool::ToolDescriptor::new(
                &def.name,
                &def.name,
                &def.description,
            )
            .with_parameters(parameters),
        }
    }
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
    request: &RunAgentRequest,
) {
    for def in request.frontend_tools() {
        tools
            .entry(def.name.clone())
            .or_insert_with(|| Arc::new(FrontendToolStub::from_agui_def(def)));
    }
}
