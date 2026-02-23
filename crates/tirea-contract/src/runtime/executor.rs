use crate::plugin::contract::AgentPlugin;
use crate::runtime::activity::ActivityManager;
use crate::runtime::control::SuspendedCall;
use crate::thread::{Message, ToolCall};
use crate::tool::contract::{Tool, ToolDescriptor, ToolResult};
use crate::RunConfig;
use async_trait::async_trait;
use futures::Stream;
use genai::chat::{ChatOptions, ChatRequest, ChatResponse, ChatStreamEvent};
use serde_json::Value;
use std::collections::HashMap;
use std::pin::Pin;
use std::sync::Arc;
use thiserror::Error;
use tirea_state::TrackedPatch;
use tokio_util::sync::CancellationToken;

/// Stream item type returned by LLM streaming executors.
pub type LlmEventStream = Pin<Box<dyn Stream<Item = genai::Result<ChatStreamEvent>> + Send>>;

/// Provider-neutral LLM execution contract consumed by the loop runtime.
#[async_trait]
pub trait LlmExecutor: Send + Sync {
    /// Execute one non-streaming chat call.
    async fn exec_chat_response(
        &self,
        model: &str,
        chat_req: ChatRequest,
        options: Option<&ChatOptions>,
    ) -> genai::Result<ChatResponse>;

    /// Execute one streaming chat call.
    async fn exec_chat_stream_events(
        &self,
        model: &str,
        chat_req: ChatRequest,
        options: Option<&ChatOptions>,
    ) -> genai::Result<LlmEventStream>;

    /// Stable executor label for debug/telemetry output.
    fn name(&self) -> &'static str {
        "llm_executor"
    }
}

/// Result of one tool call execution.
#[derive(Debug, Clone)]
pub struct ToolExecution {
    pub call: ToolCall,
    pub result: ToolResult,
    pub patch: Option<TrackedPatch>,
}

/// Canonical outcome for one tool call lifecycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolCallOutcome {
    /// Tool execution was suspended and needs external resume/decision.
    Suspended,
    /// Tool execution succeeded.
    Succeeded,
    /// Tool execution failed.
    Failed,
}

impl ToolCallOutcome {
    /// Derive outcome from a concrete `ToolResult`.
    pub fn from_tool_result(result: &ToolResult) -> Self {
        match result.status {
            crate::tool::contract::ToolStatus::Pending => Self::Suspended,
            crate::tool::contract::ToolStatus::Error => Self::Failed,
            crate::tool::contract::ToolStatus::Success
            | crate::tool::contract::ToolStatus::Warning => Self::Succeeded,
        }
    }
}

/// Input envelope passed to tool execution strategies.
pub struct ToolExecutionRequest<'a> {
    pub tools: &'a HashMap<String, Arc<dyn Tool>>,
    pub calls: &'a [ToolCall],
    pub state: &'a Value,
    pub tool_descriptors: &'a [ToolDescriptor],
    pub plugins: &'a [Arc<dyn AgentPlugin>],
    pub activity_manager: Option<Arc<dyn ActivityManager>>,
    pub run_config: &'a RunConfig,
    pub thread_id: &'a str,
    pub thread_messages: &'a [Arc<Message>],
    pub state_version: u64,
    pub cancellation_token: Option<&'a CancellationToken>,
}

/// Output item produced by tool execution strategies.
#[derive(Debug, Clone)]
pub struct ToolExecutionResult {
    pub execution: ToolExecution,
    pub outcome: ToolCallOutcome,
    /// Suspension payload for suspended outcomes.
    pub suspended_call: Option<SuspendedCall>,
    pub reminders: Vec<String>,
    pub pending_patches: Vec<TrackedPatch>,
}

/// Error returned by custom tool executors.
#[derive(Debug, Clone, Error)]
pub enum ToolExecutorError {
    #[error("tool execution cancelled")]
    Cancelled { thread_id: String },
    #[error("tool execution failed: {message}")]
    Failed { message: String },
}

/// Strategy abstraction for tool execution.
#[async_trait]
pub trait ToolExecutor: Send + Sync {
    async fn execute(
        &self,
        request: ToolExecutionRequest<'_>,
    ) -> Result<Vec<ToolExecutionResult>, ToolExecutorError>;

    /// Stable strategy label for logs/debug output.
    fn name(&self) -> &'static str;

    /// Whether apply step should enforce parallel patch conflict checks.
    fn requires_parallel_patch_conflict_check(&self) -> bool {
        false
    }
}
