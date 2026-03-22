//! Local sub-agent tool — executes a sub-agent run and returns the result.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{Value, json};

use awaken_contract::contract::tool::{
    Tool, ToolCallContext, ToolDescriptor, ToolError, ToolResult,
};

use crate::runtime::AgentResolver;

/// Tool that delegates execution to a local sub-agent.
///
/// When the LLM calls this tool, it resolves the target agent by ID,
/// and returns the agent descriptor as the tool output. The actual
/// execution is handled by the runtime's delegation infrastructure.
pub struct AgentTool {
    /// Target agent ID to delegate to.
    target_agent_id: String,
    /// Human-readable description for the LLM.
    description: String,
    /// Resolver for looking up the target agent.
    resolver: Arc<dyn AgentResolver>,
}

impl AgentTool {
    /// Create a new agent delegation tool.
    pub fn new(
        target_agent_id: impl Into<String>,
        description: impl Into<String>,
        resolver: Arc<dyn AgentResolver>,
    ) -> Self {
        let target_agent_id = target_agent_id.into();
        Self {
            description: description.into(),
            target_agent_id,
            resolver,
        }
    }

    /// Returns the target agent ID.
    pub fn target_agent_id(&self) -> &str {
        &self.target_agent_id
    }
}

#[async_trait]
impl Tool for AgentTool {
    fn descriptor(&self) -> ToolDescriptor {
        let tool_id = format!("agent_run_{}", self.target_agent_id);
        ToolDescriptor::new(&tool_id, &tool_id, &self.description)
    }

    fn validate_args(&self, args: &Value) -> Result<(), ToolError> {
        // Require a "prompt" field
        if args.get("prompt").and_then(Value::as_str).is_none() {
            return Err(ToolError::InvalidArguments(
                "missing required field \"prompt\"".into(),
            ));
        }
        Ok(())
    }

    async fn execute(&self, args: Value, _ctx: &ToolCallContext) -> Result<ToolResult, ToolError> {
        let prompt = args
            .get("prompt")
            .and_then(Value::as_str)
            .unwrap_or_default();

        // Verify the target agent exists by attempting resolution
        let resolved = self.resolver.resolve(&self.target_agent_id).map_err(|e| {
            ToolError::ExecutionFailed(format!(
                "failed to resolve agent '{}': {}",
                self.target_agent_id, e
            ))
        })?;

        let tool_id = format!("agent_run_{}", self.target_agent_id);
        Ok(ToolResult::success(
            &tool_id,
            json!({
                "agent_id": self.target_agent_id,
                "agent_name": resolved.config.id,
                "prompt": prompt,
                "status": "delegated",
                "message": format!(
                    "Delegated to agent '{}'. The sub-agent will process the request.",
                    self.target_agent_id
                ),
            }),
        ))
    }
}
