use super::tool_exec::ParallelToolExecutor;
use super::AgentLoopError;
use crate::contracts::plugin::AgentPlugin;
use crate::contracts::runtime::ToolExecutor;
use crate::contracts::tool::{Tool, ToolDescriptor};
use crate::contracts::RunContext;
use async_trait::async_trait;
use genai::chat::ChatOptions;
use genai::Client;
use std::collections::HashMap;
use std::sync::Arc;

/// Retry strategy for LLM inference calls.
#[derive(Debug, Clone)]
pub struct LlmRetryPolicy {
    /// Max attempts per model candidate (must be >= 1).
    pub max_attempts_per_model: usize,
    /// Initial backoff for retries in milliseconds.
    pub initial_backoff_ms: u64,
    /// Max backoff cap in milliseconds.
    pub max_backoff_ms: u64,
    /// Retry stream startup failures before any output is emitted.
    pub retry_stream_start: bool,
}

impl Default for LlmRetryPolicy {
    fn default() -> Self {
        Self {
            max_attempts_per_model: 2,
            initial_backoff_ms: 250,
            max_backoff_ms: 2_000,
            retry_stream_start: true,
        }
    }
}

/// Input context passed to per-step tool providers.
pub struct StepToolInput<'a> {
    /// Current run context at step boundary.
    pub state: &'a RunContext,
}

/// Tool snapshot resolved for one step.
#[derive(Clone, Default)]
pub struct StepToolSnapshot {
    /// Concrete tool map used for this step.
    pub tools: HashMap<String, Arc<dyn Tool>>,
    /// Tool descriptors exposed to plugins/LLM for this step.
    pub descriptors: Vec<ToolDescriptor>,
}

impl StepToolSnapshot {
    /// Build a step snapshot from a concrete tool map.
    pub fn from_tools(tools: HashMap<String, Arc<dyn Tool>>) -> Self {
        let descriptors = tools
            .values()
            .map(|tool| tool.descriptor().clone())
            .collect();
        Self { tools, descriptors }
    }
}

/// Provider that resolves the tool snapshot for each step.
#[async_trait]
pub trait StepToolProvider: Send + Sync {
    /// Resolve tool map + descriptors for the current step.
    async fn provide(&self, input: StepToolInput<'_>) -> Result<StepToolSnapshot, AgentLoopError>;
}

/// Boxed stream of LLM chat events.
pub type LlmEventStream = std::pin::Pin<
    Box<dyn futures::Stream<Item = Result<genai::chat::ChatStreamEvent, genai::Error>> + Send>,
>;

/// Abstraction over LLM inference backends.
///
/// The agent loop calls this trait for both non-streaming (`exec_chat_response`)
/// and streaming (`exec_chat_stream_events`) inference.  The default
/// implementation ([`GenaiLlmExecutor`]) delegates to `genai::Client`.
#[async_trait]
pub trait LlmExecutor: Send + Sync {
    /// Run a non-streaming chat completion.
    async fn exec_chat_response(
        &self,
        model: &str,
        chat_req: genai::chat::ChatRequest,
        options: Option<&genai::chat::ChatOptions>,
    ) -> genai::Result<genai::chat::ChatResponse>;

    /// Run a streaming chat completion, returning a boxed event stream.
    async fn exec_chat_stream_events(
        &self,
        model: &str,
        chat_req: genai::chat::ChatRequest,
        options: Option<&genai::chat::ChatOptions>,
    ) -> genai::Result<LlmEventStream>;

    /// Stable label for logging / debug output.
    fn name(&self) -> &'static str;
}

/// Default LLM executor backed by `genai::Client`.
#[derive(Clone)]
pub struct GenaiLlmExecutor {
    client: Client,
}

impl GenaiLlmExecutor {
    pub fn new(client: Client) -> Self {
        Self { client }
    }
}

impl std::fmt::Debug for GenaiLlmExecutor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GenaiLlmExecutor").finish()
    }
}

#[async_trait]
impl LlmExecutor for GenaiLlmExecutor {
    async fn exec_chat_response(
        &self,
        model: &str,
        chat_req: genai::chat::ChatRequest,
        options: Option<&ChatOptions>,
    ) -> genai::Result<genai::chat::ChatResponse> {
        self.client.exec_chat(model, chat_req, options).await
    }

    async fn exec_chat_stream_events(
        &self,
        model: &str,
        chat_req: genai::chat::ChatRequest,
        options: Option<&ChatOptions>,
    ) -> genai::Result<LlmEventStream> {
        let resp = self
            .client
            .exec_chat_stream(model, chat_req, options)
            .await?;
        Ok(Box::pin(resp.stream))
    }

    fn name(&self) -> &'static str {
        "genai_client"
    }
}

/// Static provider that always returns the same tool map.
#[derive(Clone, Default)]
pub struct StaticStepToolProvider {
    tools: HashMap<String, Arc<dyn Tool>>,
}

impl StaticStepToolProvider {
    pub fn new(tools: HashMap<String, Arc<dyn Tool>>) -> Self {
        Self { tools }
    }
}

#[async_trait]
impl StepToolProvider for StaticStepToolProvider {
    async fn provide(&self, _input: StepToolInput<'_>) -> Result<StepToolSnapshot, AgentLoopError> {
        Ok(StepToolSnapshot::from_tools(self.tools.clone()))
    }
}

/// Runtime configuration for the agent loop.
#[derive(Clone)]
pub struct AgentConfig {
    /// Unique identifier for this agent.
    pub id: String,
    /// Model identifier (e.g., "gpt-4", "claude-3-opus").
    pub model: String,
    /// System prompt for the LLM.
    pub system_prompt: String,
    /// Optional loop-budget hint (core loop does not enforce this directly).
    pub max_rounds: usize,
    /// Tool execution strategy (parallel, sequential, or custom).
    pub tool_executor: Arc<dyn ToolExecutor>,
    /// Chat options for the LLM.
    pub chat_options: Option<ChatOptions>,
    /// Fallback model ids used when the primary model fails.
    ///
    /// Evaluated in order after `model`.
    pub fallback_models: Vec<String>,
    /// Retry policy for LLM inference failures.
    pub llm_retry_policy: LlmRetryPolicy,
    /// Plugins to run during the agent loop.
    pub plugins: Vec<Arc<dyn AgentPlugin>>,
    /// Optional per-step tool provider.
    ///
    /// When not set, the loop uses a static provider derived from the `tools`
    /// map passed to `run_step` / `run_loop` / `run_loop_stream`.
    pub step_tool_provider: Option<Arc<dyn StepToolProvider>>,
    /// Optional LLM executor override.
    ///
    /// When not set, the loop uses [`GenaiLlmExecutor`] with `Client::default()`.
    pub llm_executor: Option<Arc<dyn LlmExecutor>>,
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            id: "default".to_string(),
            model: "gpt-4o-mini".to_string(),
            system_prompt: String::new(),
            max_rounds: 10,
            tool_executor: Arc::new(ParallelToolExecutor::streaming()),
            chat_options: Some(
                ChatOptions::default()
                    .with_capture_usage(true)
                    .with_capture_reasoning_content(true)
                    .with_capture_tool_calls(true),
            ),
            fallback_models: Vec::new(),
            llm_retry_policy: LlmRetryPolicy::default(),
            plugins: Vec::new(),
            step_tool_provider: None,
            llm_executor: None,
        }
    }
}

impl std::fmt::Debug for AgentConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AgentConfig")
            .field("id", &self.id)
            .field("model", &self.model)
            .field(
                "system_prompt",
                &format!("[{} chars]", self.system_prompt.len()),
            )
            .field("max_rounds", &self.max_rounds)
            .field("tool_executor", &self.tool_executor.name())
            .field("chat_options", &self.chat_options)
            .field("fallback_models", &self.fallback_models)
            .field("llm_retry_policy", &self.llm_retry_policy)
            .field("plugins", &format!("[{} plugins]", self.plugins.len()))
            .field(
                "step_tool_provider",
                &self.step_tool_provider.as_ref().map(|_| "<set>"),
            )
            .field(
                "llm_executor",
                &self
                    .llm_executor
                    .as_ref()
                    .map(|executor| executor.name())
                    .unwrap_or("genai_client(default)"),
            )
            .finish()
    }
}

impl AgentConfig {
    tirea_contract::impl_shared_agent_builder_methods!();
    tirea_contract::impl_loop_config_builder_methods!();

    /// Set tool executor strategy.
    #[must_use]
    pub fn with_tool_executor(mut self, executor: Arc<dyn ToolExecutor>) -> Self {
        self.tool_executor = executor;
        self
    }

    /// Set static tool map (wraps in [`StaticStepToolProvider`]).
    ///
    /// Prefer passing tools directly to [`run_loop`] / [`run_loop_stream`];
    /// use this only when you need to set tools via `step_tool_provider`.
    #[must_use]
    pub fn with_tools(self, tools: HashMap<String, Arc<dyn Tool>>) -> Self {
        self.with_step_tool_provider(Arc::new(StaticStepToolProvider::new(tools)))
    }

    /// Set per-step tool provider.
    #[must_use]
    pub fn with_step_tool_provider(mut self, provider: Arc<dyn StepToolProvider>) -> Self {
        self.step_tool_provider = Some(provider);
        self
    }

    /// Set LLM executor.
    #[must_use]
    pub fn with_llm_executor(mut self, executor: Arc<dyn LlmExecutor>) -> Self {
        self.llm_executor = Some(executor);
        self
    }

    /// Check if any plugins are configured.
    pub fn has_plugins(&self) -> bool {
        !self.plugins.is_empty()
    }
}
