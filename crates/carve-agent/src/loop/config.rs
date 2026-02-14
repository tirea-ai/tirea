use crate::plugin::AgentPlugin;
use crate::stop::{StopCondition, StopConditionSpec};
use genai::chat::ChatOptions;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

pub type RunCancellationToken = CancellationToken;

/// Definition for the agent loop configuration.
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
    /// Merge policy for scratchpad updates produced by parallel tool execution.
    ///
    /// Scratchpad keys are intentionally developer-defined. Components may share
    /// namespaces/keys by convention; this policy controls how same-key updates are
    /// resolved when multiple parallel tool calls update scratchpad in one round.
    pub scratchpad_merge_policy: ScratchpadMergePolicy,
    /// Chat options for the LLM.
    pub chat_options: Option<ChatOptions>,
    /// Plugins to run during the agent loop.
    pub plugins: Vec<Arc<dyn AgentPlugin>>,
    /// Plugin references to resolve via AgentOs wiring.
    ///
    /// This keeps AgentDefinition decoupled from plugin construction/loading. The AgentOs
    /// instance decides how to map these ids to plugin instances.
    pub plugin_ids: Vec<String>,
    /// Policy references to resolve via AgentOs wiring.
    ///
    /// Policies are "guardrails" plugins. They are resolved from the same registry as plugins,
    /// but are wired ahead of non-policy plugins to run first within the non-system plugin chain.
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
    ///
    /// When empty (and `stop_condition_specs` is also empty), a default
    /// [`crate::stop::MaxRounds`] condition is created from `max_rounds`.
    /// When non-empty, `max_rounds` is ignored.
    pub stop_conditions: Vec<Arc<dyn StopCondition>>,
    /// Declarative stop condition specs, resolved to `Arc<dyn StopCondition>`
    /// at runtime. Analogous to `plugin_ids` for plugins.
    ///
    /// Specs are appended after explicit `stop_conditions` in evaluation order.
    pub stop_condition_specs: Vec<StopConditionSpec>,
}

/// Conflict resolution policy for scratchpad updates from parallel tools.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScratchpadMergePolicy {
    /// Fail the run when parallel tool executions propose different values for the same key.
    Strict,
    /// Resolve conflicts by deterministic last-writer-wins in tool-call order.
    DeterministicLww,
}

impl Default for ScratchpadMergePolicy {
    fn default() -> Self {
        Self::DeterministicLww
    }
}

/// Backwards-compatible alias.
pub type AgentConfig = AgentDefinition;

/// Optional lifecycle context for a streaming agent run.
///
/// Run-specific data (run_id, parent_run_id, etc.) should be set on
/// `thread.runtime` before starting the loop. This struct only holds
/// the cancellation token which is orthogonal to the data model.
#[derive(Debug, Clone, Default)]
pub struct RunContext {
    /// Cancellation token for cooperative loop termination.
    ///
    /// When cancelled, this is the **run cancellation signal**:
    /// the loop stops at the next check point and emits `RunFinish` with
    /// `StopReason::Cancelled`.
    pub cancellation_token: Option<RunCancellationToken>,
}

impl RunContext {
    pub fn run_cancellation_token(&self) -> Option<&RunCancellationToken> {
        self.cancellation_token.as_ref()
    }
}

/// Runtime key: caller session id visible to tools.
pub(crate) const TOOL_RUNTIME_CALLER_THREAD_ID_KEY: &str = "__agent_tool_caller_thread_id";
/// Runtime key: caller agent id visible to tools.
pub(crate) const TOOL_RUNTIME_CALLER_AGENT_ID_KEY: &str = "__agent_tool_caller_agent_id";
/// Runtime key: caller state snapshot visible to tools.
pub(crate) const TOOL_RUNTIME_CALLER_STATE_KEY: &str = "__agent_tool_caller_state";
/// Runtime key: caller message snapshot visible to tools.
pub(crate) const TOOL_RUNTIME_CALLER_MESSAGES_KEY: &str = "__agent_tool_caller_messages";

impl Default for AgentDefinition {
    fn default() -> Self {
        Self {
            id: "default".to_string(),
            model: "gpt-4o-mini".to_string(),
            system_prompt: String::new(),
            max_rounds: 10,
            parallel_tools: true,
            scratchpad_merge_policy: ScratchpadMergePolicy::default(),
            chat_options: Some(
                ChatOptions::default()
                    .with_capture_usage(true)
                    .with_capture_tool_calls(true),
            ),
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
            .field("scratchpad_merge_policy", &self.scratchpad_merge_policy)
            .field("chat_options", &self.chat_options)
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
    /// Create a new definition with the given model.
    pub fn new(model: impl Into<String>) -> Self {
        Self {
            model: model.into(),
            ..Default::default()
        }
    }

    /// Create a new definition with explicit id and model.
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

    /// Set parallel tool execution.
    #[must_use]
    pub fn with_parallel_tools(mut self, parallel: bool) -> Self {
        self.parallel_tools = parallel;
        self
    }

    /// Set scratchpad merge policy for parallel tool execution.
    #[must_use]
    pub fn with_scratchpad_merge_policy(mut self, policy: ScratchpadMergePolicy) -> Self {
        self.scratchpad_merge_policy = policy;
        self
    }

    /// Set chat options.
    #[must_use]
    pub fn with_chat_options(mut self, options: ChatOptions) -> Self {
        self.chat_options = Some(options);
        self
    }

    /// Set plugins.
    #[must_use]
    pub fn with_plugins(mut self, plugins: Vec<Arc<dyn AgentPlugin>>) -> Self {
        self.plugins = plugins;
        self
    }

    /// Set plugin references to be resolved via AgentOs.
    #[must_use]
    pub fn with_plugin_ids(mut self, plugin_ids: Vec<String>) -> Self {
        self.plugin_ids = plugin_ids;
        self
    }

    /// Add a single plugin reference.
    #[must_use]
    pub fn with_plugin_id(mut self, plugin_id: impl Into<String>) -> Self {
        self.plugin_ids.push(plugin_id.into());
        self
    }

    /// Set policy references to be resolved via AgentOs.
    #[must_use]
    pub fn with_policy_ids(mut self, policy_ids: Vec<String>) -> Self {
        self.policy_ids = policy_ids;
        self
    }

    /// Add a single policy reference.
    #[must_use]
    pub fn with_policy_id(mut self, policy_id: impl Into<String>) -> Self {
        self.policy_ids.push(policy_id.into());
        self
    }

    /// Add a single plugin.
    #[must_use]
    pub fn with_plugin(mut self, plugin: Arc<dyn AgentPlugin>) -> Self {
        self.plugins.push(plugin);
        self
    }

    /// Set allowed tools whitelist.
    #[must_use]
    pub fn with_allowed_tools(mut self, tools: Vec<String>) -> Self {
        self.allowed_tools = Some(tools);
        self
    }

    /// Set excluded tools blacklist.
    #[must_use]
    pub fn with_excluded_tools(mut self, tools: Vec<String>) -> Self {
        self.excluded_tools = Some(tools);
        self
    }

    /// Set allowed skills whitelist.
    #[must_use]
    pub fn with_allowed_skills(mut self, skills: Vec<String>) -> Self {
        self.allowed_skills = Some(skills);
        self
    }

    /// Set excluded skills blacklist.
    #[must_use]
    pub fn with_excluded_skills(mut self, skills: Vec<String>) -> Self {
        self.excluded_skills = Some(skills);
        self
    }

    /// Set allowed delegate agents whitelist for `agent_run`.
    #[must_use]
    pub fn with_allowed_agents(mut self, agents: Vec<String>) -> Self {
        self.allowed_agents = Some(agents);
        self
    }

    /// Set excluded delegate agents blacklist for `agent_run`.
    #[must_use]
    pub fn with_excluded_agents(mut self, agents: Vec<String>) -> Self {
        self.excluded_agents = Some(agents);
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

    /// Check if any plugins are configured.
    pub fn has_plugins(&self) -> bool {
        !self.plugins.is_empty() || !self.plugin_ids.is_empty() || !self.policy_ids.is_empty()
    }
}
