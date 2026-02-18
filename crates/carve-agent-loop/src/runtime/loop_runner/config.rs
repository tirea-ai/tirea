use super::tool_exec::{ParallelToolExecutor, SequentialToolExecutor};
use super::AgentLoopError;
use crate::contracts::plugin::AgentPlugin;
use crate::contracts::runtime::{LlmExecutor, StopConditionSpec, StopPolicy, ToolExecutor};
use crate::contracts::state::AgentState;
use crate::contracts::tool::{Tool, ToolDescriptor};
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
    /// Current AgentState at step boundary.
    pub state: &'a AgentState,
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
    ) -> genai::Result<crate::contracts::runtime::LlmEventStream> {
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

/// Provider that overlays additional tools onto a base provider's snapshot.
///
/// Tools in `overlay` that do NOT exist in the base snapshot are added.
/// Tools that share a name with a base tool are silently skipped (base wins).
#[derive(Clone)]
pub struct OverlayStepToolProvider {
    base: Arc<dyn StepToolProvider>,
    overlay: HashMap<String, Arc<dyn Tool>>,
}

impl OverlayStepToolProvider {
    pub fn new(base: Arc<dyn StepToolProvider>, overlay: HashMap<String, Arc<dyn Tool>>) -> Self {
        Self { base, overlay }
    }
}

#[async_trait]
impl StepToolProvider for OverlayStepToolProvider {
    async fn provide(&self, input: StepToolInput<'_>) -> Result<StepToolSnapshot, AgentLoopError> {
        let mut snapshot = self.base.provide(input).await?;
        for (id, tool) in &self.overlay {
            if !snapshot.tools.contains_key(id) {
                snapshot.descriptors.push(tool.descriptor());
                snapshot.tools.insert(id.clone(), tool.clone());
            }
        }
        Ok(snapshot)
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
    /// Maximum number of tool call rounds before stopping.
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
    /// Composable stop policies checked after each tool-call round.
    ///
    /// When empty (and `stop_condition_specs` is also empty), a default
    /// [`crate::engine::stop_conditions::MaxRounds`] condition is created from `max_rounds`.
    /// When non-empty, `max_rounds` is ignored.
    pub stop_conditions: Vec<Arc<dyn StopPolicy>>,
    /// Declarative stop condition specs, resolved to `Arc<dyn StopPolicy>`
    /// at runtime.
    ///
    /// Specs are appended after explicit `stop_conditions` in evaluation order.
    pub stop_condition_specs: Vec<StopConditionSpec>,
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
            tool_executor: Arc::new(ParallelToolExecutor),
            chat_options: Some(
                ChatOptions::default()
                    .with_capture_usage(true)
                    .with_capture_tool_calls(true),
            ),
            fallback_models: Vec::new(),
            llm_retry_policy: LlmRetryPolicy::default(),
            plugins: Vec::new(),
            stop_conditions: Vec::new(),
            stop_condition_specs: Vec::new(),
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
                "stop_conditions",
                &format!("[{} conditions]", self.stop_conditions.len()),
            )
            .field("stop_condition_specs", &self.stop_condition_specs)
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
    carve_agent_contract::impl_shared_agent_builder_methods!();

    /// Set tool executor strategy.
    #[must_use]
    pub fn with_tool_executor(mut self, executor: Arc<dyn ToolExecutor>) -> Self {
        self.tool_executor = executor;
        self
    }

    /// Set parallel tool execution convenience strategy.
    #[must_use]
    pub fn with_parallel_tools(mut self, parallel: bool) -> Self {
        self.tool_executor = if parallel {
            Arc::new(ParallelToolExecutor)
        } else {
            Arc::new(SequentialToolExecutor)
        };
        self
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

    /// Clear per-step tool provider and use static tools input.
    #[must_use]
    pub fn clear_step_tool_provider(mut self) -> Self {
        self.step_tool_provider = None;
        self
    }

    /// Check if any plugins are configured.
    pub fn has_plugins(&self) -> bool {
        !self.plugins.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contracts::tool::{ToolError, ToolResult};
    use crate::contracts::AgentState as ContextAgentState;
    use serde_json::json;

    #[derive(Debug)]
    struct FakeTool {
        id: String,
        description: String,
    }

    impl FakeTool {
        fn new(id: &str, desc: &str) -> Self {
            Self {
                id: id.to_string(),
                description: desc.to_string(),
            }
        }
    }

    #[async_trait]
    impl Tool for FakeTool {
        fn descriptor(&self) -> ToolDescriptor {
            ToolDescriptor::new(&self.id, &self.id, &self.description)
        }

        async fn execute(
            &self,
            _args: serde_json::Value,
            _ctx: &ContextAgentState,
        ) -> Result<ToolResult, ToolError> {
            Ok(ToolResult::success(&self.id, json!({"ok": true})))
        }
    }

    fn make_base_provider(tools: Vec<(&str, &str)>) -> Arc<dyn StepToolProvider> {
        let map: HashMap<String, Arc<dyn Tool>> = tools
            .into_iter()
            .map(|(id, desc)| (id.to_string(), Arc::new(FakeTool::new(id, desc)) as Arc<dyn Tool>))
            .collect();
        Arc::new(StaticStepToolProvider::new(map))
    }

    fn make_overlay(tools: Vec<(&str, &str)>) -> HashMap<String, Arc<dyn Tool>> {
        tools
            .into_iter()
            .map(|(id, desc)| (id.to_string(), Arc::new(FakeTool::new(id, desc)) as Arc<dyn Tool>))
            .collect()
    }

    fn dummy_state() -> crate::contracts::state::AgentState {
        crate::contracts::state::AgentState::with_initial_state("test", json!({}))
    }

    #[tokio::test]
    async fn overlay_provider_adds_new_tools() {
        let base = make_base_provider(vec![("backend_tool", "base")]);
        let overlay = make_overlay(vec![("frontend_tool", "overlay")]);
        let provider = OverlayStepToolProvider::new(base, overlay);

        let snapshot = provider
            .provide(StepToolInput { state: &dummy_state() })
            .await
            .unwrap();

        assert!(snapshot.tools.contains_key("backend_tool"));
        assert!(snapshot.tools.contains_key("frontend_tool"));
        assert_eq!(snapshot.tools.len(), 2);
        assert_eq!(snapshot.descriptors.len(), 2);
    }

    #[tokio::test]
    async fn overlay_provider_skips_shadowed_tools() {
        let base = make_base_provider(vec![("shared_tool", "base version")]);
        let overlay = make_overlay(vec![("shared_tool", "overlay version")]);
        let provider = OverlayStepToolProvider::new(base, overlay);

        let snapshot = provider
            .provide(StepToolInput { state: &dummy_state() })
            .await
            .unwrap();

        assert_eq!(snapshot.tools.len(), 1);
        assert_eq!(snapshot.descriptors.len(), 1);
        // Base tool wins — check description
        let desc = &snapshot.descriptors[0];
        assert_eq!(desc.description, "base version");
    }

    #[tokio::test]
    async fn overlay_provider_empty_overlay() {
        let base = make_base_provider(vec![("tool_a", "a"), ("tool_b", "b")]);
        let overlay = HashMap::new();
        let provider = OverlayStepToolProvider::new(base, overlay);

        let snapshot = provider
            .provide(StepToolInput { state: &dummy_state() })
            .await
            .unwrap();

        assert_eq!(snapshot.tools.len(), 2);
        assert_eq!(snapshot.descriptors.len(), 2);
    }
}
