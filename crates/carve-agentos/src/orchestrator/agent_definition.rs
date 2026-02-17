use crate::contracts::plugin::AgentPlugin;
use crate::contracts::runtime::StopConditionSpec;
use crate::engine::stop_conditions::StopCondition;
use crate::runtime::loop_runner::{
    AgentConfig, LlmRetryPolicy, ParallelToolExecutor, SequentialToolExecutor,
};
use genai::chat::ChatOptions;
use std::sync::Arc;

/// Agent composition definition owned by AgentOS orchestration.
///
/// This is the orchestration-facing model and can include registry references
/// (`plugin_ids`, `policy_ids`) and policy filters (`allowed_*`, `excluded_*`).
/// Before execution, AgentOS resolves it into loop-facing [`AgentConfig`].
#[derive(Clone)]
pub struct AgentDefinition {
    /// Unique identifier for this agent.
    pub id: String,
    /// Model identifier (e.g., "gpt-4", "claude-3-opus").
    pub model: String,
    /// System prompt for the LLM.
    pub system_prompt: String,
    /// Maximum number of tool call rounds before stopping.
    pub max_rounds: usize,
    /// Whether to execute tools in parallel.
    pub parallel_tools: bool,
    /// Chat options for the LLM.
    pub chat_options: Option<ChatOptions>,
    /// Fallback model ids used when the primary model fails.
    ///
    /// Evaluated in order after `model`.
    pub fallback_models: Vec<String>,
    /// Retry policy for LLM inference failures.
    pub llm_retry_policy: LlmRetryPolicy,
    /// Explicit plugin instances.
    pub plugins: Vec<Arc<dyn AgentPlugin>>,
    /// Plugin references resolved from AgentOS plugin registry.
    pub plugin_ids: Vec<String>,
    /// Policy references resolved from AgentOS plugin registry.
    pub policy_ids: Vec<String>,
    /// Tool whitelist (None = all tools available).
    pub allowed_tools: Option<Vec<String>>,
    /// Tool blacklist.
    pub excluded_tools: Option<Vec<String>>,
    /// Skill whitelist (None = all skills available).
    pub allowed_skills: Option<Vec<String>>,
    /// Skill blacklist.
    pub excluded_skills: Option<Vec<String>>,
    /// Agent whitelist for `agent_run` delegation (None = all visible agents available).
    pub allowed_agents: Option<Vec<String>>,
    /// Agent blacklist for `agent_run` delegation.
    pub excluded_agents: Option<Vec<String>>,
    /// Composable stop conditions checked after each tool-call round.
    pub stop_conditions: Vec<Arc<dyn StopCondition>>,
    /// Declarative stop condition specs, resolved at runtime.
    pub stop_condition_specs: Vec<StopConditionSpec>,
}

impl Default for AgentDefinition {
    fn default() -> Self {
        Self {
            id: "default".to_string(),
            model: "gpt-4o-mini".to_string(),
            system_prompt: String::new(),
            max_rounds: 10,
            parallel_tools: true,
            chat_options: Some(
                ChatOptions::default()
                    .with_capture_usage(true)
                    .with_capture_tool_calls(true),
            ),
            fallback_models: Vec::new(),
            llm_retry_policy: LlmRetryPolicy::default(),
            plugins: Vec::new(),
            plugin_ids: Vec::new(),
            policy_ids: Vec::new(),
            allowed_tools: None,
            excluded_tools: None,
            allowed_skills: None,
            excluded_skills: None,
            allowed_agents: None,
            excluded_agents: None,
            stop_conditions: Vec::new(),
            stop_condition_specs: Vec::new(),
        }
    }
}

impl std::fmt::Debug for AgentDefinition {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AgentDefinition")
            .field("id", &self.id)
            .field("model", &self.model)
            .field(
                "system_prompt",
                &format!("[{} chars]", self.system_prompt.len()),
            )
            .field("max_rounds", &self.max_rounds)
            .field("parallel_tools", &self.parallel_tools)
            .field("chat_options", &self.chat_options)
            .field("fallback_models", &self.fallback_models)
            .field("llm_retry_policy", &self.llm_retry_policy)
            .field("plugins", &format!("[{} plugins]", self.plugins.len()))
            .field("plugin_ids", &self.plugin_ids)
            .field("policy_ids", &self.policy_ids)
            .field("allowed_tools", &self.allowed_tools)
            .field("excluded_tools", &self.excluded_tools)
            .field("allowed_skills", &self.allowed_skills)
            .field("excluded_skills", &self.excluded_skills)
            .field("allowed_agents", &self.allowed_agents)
            .field("excluded_agents", &self.excluded_agents)
            .field(
                "stop_conditions",
                &format!("[{} conditions]", self.stop_conditions.len()),
            )
            .field("stop_condition_specs", &self.stop_condition_specs)
            .finish()
    }
}

impl AgentDefinition {
    /// Create a new agent definition with the given model.
    pub fn new(model: impl Into<String>) -> Self {
        Self {
            model: model.into(),
            ..Default::default()
        }
    }

    /// Create a new agent definition with explicit id and model.
    pub fn with_id(id: impl Into<String>, model: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            model: model.into(),
            ..Default::default()
        }
    }

    #[must_use]
    pub fn with_system_prompt(mut self, prompt: impl Into<String>) -> Self {
        self.system_prompt = prompt.into();
        self
    }

    #[must_use]
    pub fn with_max_rounds(mut self, max_rounds: usize) -> Self {
        self.max_rounds = max_rounds;
        self
    }

    #[must_use]
    pub fn with_parallel_tools(mut self, parallel: bool) -> Self {
        self.parallel_tools = parallel;
        self
    }

    #[must_use]
    pub fn with_chat_options(mut self, options: ChatOptions) -> Self {
        self.chat_options = Some(options);
        self
    }

    #[must_use]
    pub fn with_fallback_models(mut self, models: Vec<String>) -> Self {
        self.fallback_models = models;
        self
    }

    #[must_use]
    pub fn with_fallback_model(mut self, model: impl Into<String>) -> Self {
        self.fallback_models.push(model.into());
        self
    }

    #[must_use]
    pub fn with_llm_retry_policy(mut self, policy: LlmRetryPolicy) -> Self {
        self.llm_retry_policy = policy;
        self
    }

    #[must_use]
    pub fn with_plugins(mut self, plugins: Vec<Arc<dyn AgentPlugin>>) -> Self {
        self.plugins = plugins;
        self
    }

    #[must_use]
    pub fn with_plugin_ids(mut self, plugin_ids: Vec<String>) -> Self {
        self.plugin_ids = plugin_ids;
        self
    }

    #[must_use]
    pub fn with_plugin_id(mut self, plugin_id: impl Into<String>) -> Self {
        self.plugin_ids.push(plugin_id.into());
        self
    }

    #[must_use]
    pub fn with_policy_ids(mut self, policy_ids: Vec<String>) -> Self {
        self.policy_ids = policy_ids;
        self
    }

    #[must_use]
    pub fn with_policy_id(mut self, policy_id: impl Into<String>) -> Self {
        self.policy_ids.push(policy_id.into());
        self
    }

    #[must_use]
    pub fn with_plugin(mut self, plugin: Arc<dyn AgentPlugin>) -> Self {
        self.plugins.push(plugin);
        self
    }

    #[must_use]
    pub fn with_allowed_tools(mut self, tools: Vec<String>) -> Self {
        self.allowed_tools = Some(tools);
        self
    }

    #[must_use]
    pub fn with_excluded_tools(mut self, tools: Vec<String>) -> Self {
        self.excluded_tools = Some(tools);
        self
    }

    #[must_use]
    pub fn with_allowed_skills(mut self, skills: Vec<String>) -> Self {
        self.allowed_skills = Some(skills);
        self
    }

    #[must_use]
    pub fn with_excluded_skills(mut self, skills: Vec<String>) -> Self {
        self.excluded_skills = Some(skills);
        self
    }

    #[must_use]
    pub fn with_allowed_agents(mut self, agents: Vec<String>) -> Self {
        self.allowed_agents = Some(agents);
        self
    }

    #[must_use]
    pub fn with_excluded_agents(mut self, agents: Vec<String>) -> Self {
        self.excluded_agents = Some(agents);
        self
    }

    #[must_use]
    pub fn with_stop_condition(mut self, condition: impl StopCondition + 'static) -> Self {
        self.stop_conditions.push(Arc::new(condition));
        self
    }

    #[must_use]
    pub fn with_stop_conditions(mut self, conditions: Vec<Arc<dyn StopCondition>>) -> Self {
        self.stop_conditions = conditions;
        self
    }

    #[must_use]
    pub fn with_stop_condition_spec(mut self, spec: StopConditionSpec) -> Self {
        self.stop_condition_specs.push(spec);
        self
    }

    #[must_use]
    pub fn with_stop_condition_specs(mut self, specs: Vec<StopConditionSpec>) -> Self {
        self.stop_condition_specs = specs;
        self
    }

    pub fn into_loop_config(self) -> AgentConfig {
        let tool_executor: Arc<dyn crate::runtime::loop_runner::ToolExecutor> =
            if self.parallel_tools {
                Arc::new(ParallelToolExecutor)
            } else {
                Arc::new(SequentialToolExecutor)
            };
        AgentConfig {
            id: self.id,
            model: self.model,
            system_prompt: self.system_prompt,
            max_rounds: self.max_rounds,
            tool_executor,
            chat_options: self.chat_options,
            fallback_models: self.fallback_models,
            llm_retry_policy: self.llm_retry_policy,
            plugins: self.plugins,
            stop_conditions: self.stop_conditions,
            stop_condition_specs: self.stop_condition_specs,
            step_tool_provider: None,
            llm_executor: None,
        }
    }
}
