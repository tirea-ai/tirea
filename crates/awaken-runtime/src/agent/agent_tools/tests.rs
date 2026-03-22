use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{Value, json};

use awaken_contract::StateError;
use awaken_contract::contract::executor::{InferenceExecutionError, InferenceRequest};
use awaken_contract::contract::inference::{StopReason, StreamResult, TokenUsage};
use awaken_contract::contract::tool::{
    Tool, ToolCallContext, ToolDescriptor, ToolError, ToolResult,
};
use awaken_contract::registry_spec::AgentSpec;

use crate::agent::config::AgentConfig;
use crate::agent::executor::SequentialToolExecutor;
use crate::runtime::{AgentResolver, ExecutionEnv, ResolvedAgent};

use super::agent_tool::AgentTool;
use super::remote_a2a::{A2aEndpoint, RemoteA2aTool};

// -- Mock Resolver --

struct MockResolver {
    agents: std::collections::HashMap<String, AgentSpec>,
}

impl MockResolver {
    fn with_agent(id: &str) -> Self {
        let mut agents = std::collections::HashMap::new();
        agents.insert(
            id.to_string(),
            AgentSpec {
                id: id.into(),
                model: "test-model".into(),
                system_prompt: "sys".into(),
                ..Default::default()
            },
        );
        Self { agents }
    }
}

struct MockExecutor;

#[async_trait]
impl awaken_contract::contract::executor::LlmExecutor for MockExecutor {
    async fn execute(
        &self,
        _request: InferenceRequest,
    ) -> Result<StreamResult, InferenceExecutionError> {
        Ok(StreamResult {
            content: vec![],
            tool_calls: vec![],
            usage: Some(TokenUsage::default()),
            stop_reason: Some(StopReason::EndTurn),
            has_incomplete_tool_calls: false,
        })
    }

    fn name(&self) -> &str {
        "mock"
    }
}

impl AgentResolver for MockResolver {
    fn resolve(&self, agent_id: &str) -> Result<ResolvedAgent, StateError> {
        let spec = self.agents.get(agent_id).ok_or(StateError::ResolveFailed {
            message: format!("agent not found: {}", agent_id),
        })?;
        Ok(ResolvedAgent {
            config: AgentConfig::new(
                &spec.id,
                &spec.model,
                &spec.system_prompt,
                Arc::new(MockExecutor),
            ),
            env: ExecutionEnv::empty(),
        })
    }
}

// -- AgentTool tests --

#[tokio::test]
async fn agent_tool_descriptor_includes_target_id() {
    let resolver = Arc::new(MockResolver::with_agent("worker"));
    let tool = AgentTool::new("worker", "Delegate to worker agent", resolver);
    let desc = tool.descriptor();
    assert_eq!(desc.id, "agent_run_worker");
    assert!(desc.description.contains("worker"));
}

#[tokio::test]
async fn agent_tool_validates_prompt() {
    let resolver = Arc::new(MockResolver::with_agent("worker"));
    let tool = AgentTool::new("worker", "desc", resolver);

    let err = tool.validate_args(&json!({}));
    assert!(err.is_err());

    let ok = tool.validate_args(&json!({"prompt": "hello"}));
    assert!(ok.is_ok());
}

#[tokio::test]
async fn agent_tool_execute_resolves_agent() {
    let resolver = Arc::new(MockResolver::with_agent("worker"));
    let tool = AgentTool::new("worker", "desc", resolver);
    let ctx = ToolCallContext::test_default();

    let result = tool
        .execute(json!({"prompt": "do work"}), &ctx)
        .await
        .unwrap();

    assert!(result.is_success());
    assert_eq!(result.data["agent_id"], "worker");
    assert_eq!(result.data["status"], "delegated");
}

#[tokio::test]
async fn agent_tool_execute_fails_for_missing_agent() {
    let resolver = Arc::new(MockResolver::with_agent("other"));
    let tool = AgentTool::new("missing", "desc", resolver);
    let ctx = ToolCallContext::test_default();

    let err = tool.execute(json!({"prompt": "do work"}), &ctx).await;
    assert!(err.is_err());
}

#[test]
fn agent_tool_target_id() {
    let resolver = Arc::new(MockResolver::with_agent("worker"));
    let tool = AgentTool::new("worker", "desc", resolver);
    assert_eq!(tool.target_agent_id(), "worker");
}

// -- RemoteA2aTool tests --

#[test]
fn remote_a2a_tool_descriptor() {
    let endpoint = A2aEndpoint::new("https://api.example.com", "remote-worker");
    let tool = RemoteA2aTool::new("remote_worker", "Remote worker agent", endpoint);
    let desc = tool.descriptor();
    assert_eq!(desc.id, "remote_worker");
    assert!(desc.description.contains("Remote worker"));
}

#[test]
fn remote_a2a_tool_validates_prompt() {
    let endpoint = A2aEndpoint::new("https://api.example.com", "worker");
    let tool = RemoteA2aTool::new("rw", "desc", endpoint);

    assert!(tool.validate_args(&json!({})).is_err());
    assert!(tool.validate_args(&json!({"prompt": "go"})).is_ok());
}

#[tokio::test]
async fn remote_a2a_tool_execute_returns_submission() {
    let endpoint = A2aEndpoint::new("https://api.example.com", "worker")
        .with_bearer_token("secret")
        .with_poll_interval_ms(1000);
    let tool = RemoteA2aTool::new("rw", "desc", endpoint);
    let ctx = ToolCallContext::test_default();

    let result = tool
        .execute(json!({"prompt": "analyze data"}), &ctx)
        .await
        .unwrap();

    assert!(result.is_success());
    assert_eq!(result.data["remote_agent_id"], "worker");
    assert_eq!(result.data["status"], "submitted");
    assert_eq!(result.data["prompt"], "analyze data");
}

#[tokio::test]
async fn remote_a2a_tool_rejects_empty_prompt() {
    let endpoint = A2aEndpoint::new("https://api.example.com", "worker");
    let tool = RemoteA2aTool::new("rw", "desc", endpoint);
    let ctx = ToolCallContext::test_default();

    let err = tool.execute(json!({"prompt": "   "}), &ctx).await;
    assert!(err.is_err());
}

#[test]
fn a2a_endpoint_builder() {
    let endpoint = A2aEndpoint::new("https://example.com/a2a", "agent-1")
        .with_bearer_token("tok_123")
        .with_poll_interval_ms(5000);

    assert_eq!(endpoint.base_url, "https://example.com/a2a");
    assert_eq!(endpoint.remote_agent_id, "agent-1");
    assert_eq!(endpoint.bearer_token.as_deref(), Some("tok_123"));
    assert_eq!(endpoint.poll_interval_ms, 5000);
}
