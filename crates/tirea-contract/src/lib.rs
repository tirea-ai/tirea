//! Shared agent contracts for conversation state, runtime protocol, extension SPI, and storage.
#![allow(missing_docs)]

/// Builder methods for pure-data fields shared by both AgentConfig and AgentDefinition.
///
/// Covers: id, model, system_prompt, max_rounds, chat_options, fallback_models,
/// llm_retry_policy, stop_condition_specs.
#[macro_export]
macro_rules! impl_shared_agent_builder_methods {
    () => {
        /// Create a new instance with the given model id.
        pub fn new(model: impl Into<String>) -> Self {
            Self {
                model: model.into(),
                ..Default::default()
            }
        }

        /// Create a new instance with explicit id and model.
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

        /// Add a declarative stop policy spec.
        #[must_use]
        pub fn with_stop_condition_spec(mut self, spec: StopConditionSpec) -> Self {
            self.stop_condition_specs.push(spec);
            self
        }

        /// Set all declarative stop policy specs, replacing any previously set.
        #[must_use]
        pub fn with_stop_condition_specs(mut self, specs: Vec<StopConditionSpec>) -> Self {
            self.stop_condition_specs = specs;
            self
        }
    };
}

/// Builder methods for runtime instance fields (plugins, stop_conditions).
///
/// Only used by AgentConfig (loop layer) which directly holds plugin and
/// stop_condition instances. AgentDefinition uses id-based references instead.
#[macro_export]
macro_rules! impl_loop_config_builder_methods {
    () => {
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

        /// Add a stop policy.
        #[must_use]
        pub fn with_stop_condition(mut self, condition: impl StopPolicy + 'static) -> Self {
            self.stop_conditions.push(Arc::new(condition));
            self
        }

        /// Set all stop policies, replacing any previously set.
        #[must_use]
        pub fn with_stop_conditions(mut self, conditions: Vec<Arc<dyn StopPolicy>>) -> Self {
            self.stop_conditions = conditions;
            self
        }
    };
}

#[cfg(any(test, feature = "test-support"))]
pub mod testing;

pub mod event;
pub mod plugin;
pub mod protocol;
pub mod runtime;
pub mod storage;
pub mod thread;
pub mod tool;

/// Per-run configuration — a business alias for the generic `SealedState` container.
pub type RunConfig = tirea_state::SealedState;

/// Error type for `RunConfig` operations.
pub type RunConfigError = tirea_state::SealedStateError;

// thread
pub use thread::{
    gen_message_id, CheckpointReason, Message, MessageMetadata, PendingDelta, Role, Thread,
    ThreadChangeSet, ThreadMetadata, ToolCall, Version, Visibility,
};

// event
pub use event::{
    AgentEvent, FrontendToolInvocation, Interaction, InteractionResponse, InvocationOrigin,
    ResponseRouting, StopConditionSpec, StopReason, TerminationReason,
};

// tool
pub use tool::{
    ActivityContext, Tool, ToolCallContext, ToolDescriptor, ToolError, ToolRegistry,
    ToolRegistryError, ToolResult, ToolStatus,
};

// plugin
pub use plugin::{AgentPlugin, Phase, PhasePolicy, StepContext, StepOutcome, ToolContext};

// runtime
pub use runtime::{
    ActivityManager, InferenceError, LlmExecutor, LoopControlExt, LoopControlState, RunContext,
    RunDelta, StopPolicy, StopPolicyInput, StopPolicyStats, StreamResult, ToolExecution,
    ToolExecutionRequest, ToolExecutionResult, ToolExecutor, ToolExecutorError,
};

// storage
pub use storage::{
    paginate_in_memory, AgentStateHead, AgentStateListPage, AgentStateListQuery, AgentStateReader,
    AgentStateStore, AgentStateStoreError, AgentStateSync, AgentStateWriter, Committed,
    MessagePage, MessageQuery, MessageWithCursor, SortOrder, VersionPrecondition,
};

// protocol
pub use protocol::{
    ProtocolHistoryEncoder, ProtocolInputAdapter, ProtocolOutputEncoder, RunRequest,
};
