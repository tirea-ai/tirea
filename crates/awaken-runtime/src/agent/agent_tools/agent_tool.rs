//! Local sub-agent tool — executes a sub-agent run and returns the result.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{Value, json};

use awaken_contract::contract::event_sink::NullEventSink;
use awaken_contract::contract::identity::RunIdentity;
use awaken_contract::contract::message::Message;
use awaken_contract::contract::tool::{
    Tool, ToolCallContext, ToolDescriptor, ToolError, ToolResult,
};

use crate::agent::loop_runner::run_agent_loop;
use crate::runtime::AgentResolver;

/// Tool that delegates execution to a local sub-agent.
///
/// When the LLM calls this tool, it resolves the target agent by ID,
/// runs a full agent loop with the given prompt, and returns the
/// sub-agent's final response as the tool output.
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

    async fn execute(&self, args: Value, ctx: &ToolCallContext) -> Result<ToolResult, ToolError> {
        let prompt = args
            .get("prompt")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim()
            .to_string();

        if prompt.is_empty() {
            return Err(ToolError::InvalidArguments(
                "prompt must not be empty".into(),
            ));
        }

        let tool_id = format!("agent_run_{}", self.target_agent_id);

        // Resolve the target agent
        let resolved = self.resolver.resolve(&self.target_agent_id).map_err(|e| {
            ToolError::ExecutionFailed(format!(
                "failed to resolve agent '{}': {}",
                self.target_agent_id, e
            ))
        })?;

        // Build execution environment
        let store = crate::state::StateStore::new();
        store
            .install_plugin(crate::agent::loop_runner::LoopStatePlugin)
            .map_err(|e| ToolError::ExecutionFailed(format!("state setup failed: {e}")))?;

        let phase_runtime = crate::runtime::PhaseRuntime::new(store.clone())
            .map_err(|e| ToolError::ExecutionFailed(format!("phase setup failed: {e}")))?;

        // Create sub-agent run identity
        let sub_run_id = uuid::Uuid::now_v7().to_string();
        let sub_identity = RunIdentity::new(
            ctx.run_identity.thread_id.clone(),
            Some(ctx.run_identity.thread_id.clone()),
            sub_run_id,
            Some(ctx.run_identity.run_id.clone()),
            self.target_agent_id.clone(),
            awaken_contract::contract::identity::RunOrigin::Subagent,
        );

        // Execute sub-agent loop with the prompt as a user message
        let sub_messages = vec![Message::user(&prompt)];
        let sink = NullEventSink;

        let result = run_agent_loop(
            self.resolver.as_ref(),
            &self.target_agent_id,
            &phase_runtime,
            &sink,
            None, // no checkpoint store for sub-agent
            sub_messages,
            sub_identity,
            None, // no cancellation token
        )
        .await
        .map_err(|e| {
            ToolError::ExecutionFailed(format!(
                "sub-agent '{}' execution failed: {}",
                self.target_agent_id, e
            ))
        })?;

        Ok(ToolResult::success(
            &tool_id,
            json!({
                "agent_id": self.target_agent_id,
                "agent_name": resolved.config.id,
                "status": format!("{:?}", result.termination).to_lowercase(),
                "response": result.response,
                "steps": result.steps,
            }),
        ))
    }
}
