use crate::plugin::contract::AgentPlugin;
use crate::event::interaction::Interaction;
use crate::runtime::activity::ActivityManager;
use crate::thread::{Message, ToolCall};
use crate::tool::contract::{Tool, ToolDescriptor, ToolResult};
use async_trait::async_trait;
use crate::RunConfig;
use carve_state::{Op, TrackedPatch};
use futures::Stream;
use genai::chat::{ChatOptions, ChatRequest, ChatResponse, ChatStreamEvent};
use serde_json::Value;
use std::collections::HashMap;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use thiserror::Error;
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

/// Input envelope passed to tool execution strategies.
pub struct ToolExecutionRequest<'a> {
    pub tools: &'a HashMap<String, Arc<dyn Tool>>,
    pub calls: &'a [ToolCall],
    pub state: &'a Value,
    pub tool_descriptors: &'a [ToolDescriptor],
    pub plugins: &'a [Arc<dyn AgentPlugin>],
    pub activity_manager: Option<Arc<dyn ActivityManager>>,
    pub run_config: Option<&'a RunConfig>,
    pub thread_id: &'a str,
    pub thread_messages: &'a [Arc<Message>],
    pub state_version: u64,
    pub cancellation_token: Option<&'a CancellationToken>,
    pub run_patch: Arc<Mutex<Vec<Op>>>,
}

/// Output item produced by tool execution strategies.
#[derive(Debug, Clone)]
pub struct ToolExecutionResult {
    pub execution: ToolExecution,
    pub reminders: Vec<String>,
    pub pending_interaction: Option<Interaction>,
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
