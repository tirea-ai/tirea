use super::tool_exec::{ParallelToolExecutor, SequentialToolExecutor, ToolExecutor};
use super::AgentLoopError;
use crate::contracts::plugin::AgentPlugin;
use crate::contracts::runtime::StopConditionSpec;
use crate::contracts::state::AgentState;
use crate::contracts::tool::{Tool, ToolDescriptor};
use crate::engine::stop_conditions::StopCondition;
use async_trait::async_trait;
use genai::chat::ChatOptions;
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
    pub thread: &'a AgentState,
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
    /// Composable stop conditions checked after each tool-call round.
    ///
    /// When empty (and `stop_condition_specs` is also empty), a default
    /// [`crate::engine::stop_conditions::MaxRounds`] condition is created from `max_rounds`.
    /// When non-empty, `max_rounds` is ignored.
    pub stop_conditions: Vec<Arc<dyn StopCondition>>,
    /// Declarative stop condition specs, resolved to `Arc<dyn StopCondition>`
    /// at runtime.
    ///
    /// Specs are appended after explicit `stop_conditions` in evaluation order.
    pub stop_condition_specs: Vec<StopConditionSpec>,
    /// Optional per-step tool provider.
    ///
    /// When not set, the loop uses a static provider derived from the `tools`
    /// map passed to `run_step` / `run_loop` / `run_loop_stream`.
    pub step_tool_provider: Option<Arc<dyn StepToolProvider>>,
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
            .finish()
    }
}

impl AgentConfig {
    /// Create a new agent config with the given model.
    pub fn new(model: impl Into<String>) -> Self {
        Self {
            model: model.into(),
            ..Default::default()
        }
    }

    /// Create a new agent config with explicit id and model.
    pub fn with_id(id: impl Into<String>, model: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            model: model.into(),
            ..Default::default()
        }
    }

    /// Set system prompt.
    #[must_use]
    pub fn with_system_prompt(mut self, prompt: impl Into<String>) -> Self {
        self.system_prompt = prompt.into();
        self
    }

    /// Set max rounds.
    #[must_use]
    pub fn with_max_rounds(mut self, max_rounds: usize) -> Self {
        self.max_rounds = max_rounds;
        self
    }

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

    /// Set chat options.
    #[must_use]
    pub fn with_chat_options(mut self, options: ChatOptions) -> Self {
        self.chat_options = Some(options);
        self
    }

    /// Set fallback model ids to try after the primary model.
    #[must_use]
    pub fn with_fallback_models(mut self, models: Vec<String>) -> Self {
        self.fallback_models = models;
        self
    }

    /// Add a single fallback model id.
    #[must_use]
    pub fn with_fallback_model(mut self, model: impl Into<String>) -> Self {
        self.fallback_models.push(model.into());
        self
    }

    /// Set LLM retry policy.
    #[must_use]
    pub fn with_llm_retry_policy(mut self, policy: LlmRetryPolicy) -> Self {
        self.llm_retry_policy = policy;
        self
    }

    /// Set plugins.
    #[must_use]
    pub fn with_plugins(mut self, plugins: Vec<Arc<dyn AgentPlugin>>) -> Self {
        self.plugins = plugins;
        self
    }

    /// Add a single plugin.
    #[must_use]
    pub fn with_plugin(mut self, plugin: Arc<dyn AgentPlugin>) -> Self {
        self.plugins.push(plugin);
        self
    }

    /// Add a stop condition.
    ///
    /// When any stop conditions are set, the `max_rounds` field is ignored
    /// and only explicit stop conditions are checked.
    #[must_use]
    pub fn with_stop_condition(mut self, condition: impl StopCondition + 'static) -> Self {
        self.stop_conditions.push(Arc::new(condition));
        self
    }

    /// Set all stop conditions, replacing any previously set.
    #[must_use]
    pub fn with_stop_conditions(mut self, conditions: Vec<Arc<dyn StopCondition>>) -> Self {
        self.stop_conditions = conditions;
        self
    }

    /// Add a declarative stop condition spec.
    ///
    /// Specs are resolved to `Arc<dyn StopCondition>` at runtime and
    /// appended after explicit `stop_conditions` in evaluation order.
    #[must_use]
    pub fn with_stop_condition_spec(mut self, spec: StopConditionSpec) -> Self {
        self.stop_condition_specs.push(spec);
        self
    }

    /// Set all declarative stop condition specs, replacing any previously set.
    #[must_use]
    pub fn with_stop_condition_specs(mut self, specs: Vec<StopConditionSpec>) -> Self {
        self.stop_condition_specs = specs;
        self
    }

    /// Set per-step tool provider.
    #[must_use]
    pub fn with_step_tool_provider(mut self, provider: Arc<dyn StepToolProvider>) -> Self {
        self.step_tool_provider = Some(provider);
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
