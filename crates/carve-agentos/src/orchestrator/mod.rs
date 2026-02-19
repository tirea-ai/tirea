use std::collections::HashMap;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use futures::Stream;
use genai::Client;

use crate::contracts::plugin::AgentPlugin;
use crate::contracts::runtime::{AgentEvent, RunRequest};
use crate::contracts::state::Thread;
use crate::contracts::state::CheckpointReason;
use crate::contracts::state::Message;
use crate::contracts::storage::{
    AgentStateHead, AgentStateStore, AgentStateStoreError, VersionPrecondition,
};
use crate::contracts::tool::Tool;
use crate::extensions::skills::{
    CompositeSkillRegistry, InMemorySkillRegistry, Skill, SkillDiscoveryPlugin, SkillError,
    SkillPlugin, SkillRegistry, SkillRegistryError, SkillRegistryManagerError, SkillRuntimePlugin,
    SkillSubsystem, SkillSubsystemError,
};
use crate::runtime::loop_runner::{
    run_loop_stream_with_input, AgentConfig, AgentLoopError, LoopRunInput, RunServices,
    StateCommitError, StateCommitter,
};

mod agent_definition;
pub(crate) mod agent_tools;
mod builder;
mod composition;
mod policy;
mod run;
mod wiring;

#[cfg(test)]
mod tests;

pub use agent_definition::AgentDefinition;
use agent_tools::{
    AgentRecoveryPlugin, AgentRunManager, AgentRunTool, AgentStopTool, AgentToolsPlugin,
};
pub use composition::{
    AgentRegistry, AgentRegistryError, BundleComposeError, BundleComposer,
    BundleRegistryAccumulator, BundleRegistryKind, CompositeAgentRegistry, CompositeModelRegistry,
    CompositePluginRegistry, CompositeProviderRegistry, CompositeToolRegistry,
    InMemoryAgentRegistry, InMemoryModelRegistry, InMemoryPluginRegistry, InMemoryProviderRegistry,
    InMemoryToolRegistry, ModelDefinition, ModelRegistry, ModelRegistryError, PluginRegistry,
    PluginRegistryError, ProviderRegistry, ProviderRegistryError, RegistryBundle, RegistrySet,
    ToolPluginBundle, ToolRegistry, ToolRegistryError,
};

type ResolvedAgentWiring = (AgentConfig, HashMap<String, Arc<dyn Tool>>, Thread);



#[derive(Clone)]
struct AgentStateStoreStateCommitter {
    agent_state_store: Arc<dyn AgentStateStore>,
}

impl AgentStateStoreStateCommitter {
    fn new(agent_state_store: Arc<dyn AgentStateStore>) -> Self {
        Self { agent_state_store }
    }
}

#[async_trait::async_trait]
impl StateCommitter for AgentStateStoreStateCommitter {
    async fn commit(
        &self,
        thread_id: &str,
        changeset: crate::contracts::AgentChangeSet,
        precondition: VersionPrecondition,
    ) -> Result<u64, StateCommitError> {
        self.agent_state_store
            .append(thread_id, &changeset, precondition)
            .await
            .map(|committed| committed.version)
            .map_err(|e| StateCommitError::new(format!("checkpoint append failed: {e}")))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkillsMode {
    Disabled,
    DiscoveryAndRuntime,
    DiscoveryOnly,
    RuntimeOnly,
}

#[derive(Debug, Clone)]
pub struct SkillsConfig {
    pub mode: SkillsMode,
    pub discovery_max_entries: usize,
    pub discovery_max_chars: usize,
}

impl Default for SkillsConfig {
    fn default() -> Self {
        Self {
            // Skills are opt-in. If enabled, the caller must provide skills.
            mode: SkillsMode::Disabled,
            discovery_max_entries: 32,
            discovery_max_chars: 16 * 1024,
        }
    }
}

#[derive(Debug, Clone)]
pub struct AgentToolsConfig {
    pub discovery_max_entries: usize,
    pub discovery_max_chars: usize,
}

impl Default for AgentToolsConfig {
    fn default() -> Self {
        Self {
            discovery_max_entries: 64,
            discovery_max_chars: 16 * 1024,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum AgentOsWiringError {
    #[error("reserved plugin id cannot be used: {0}")]
    ReservedPluginId(String),

    #[error("plugin not found: {0}")]
    PluginNotFound(String),

    #[error("plugin id already installed: {0}")]
    PluginAlreadyInstalled(String),

    #[error("skills tool id already registered: {0}")]
    SkillsToolIdConflict(String),

    #[error("skills plugin already installed: {0}")]
    SkillsPluginAlreadyInstalled(String),

    #[error("skills enabled but no skills configured")]
    SkillsNotConfigured,

    #[error("agent tool id already registered: {0}")]
    AgentToolIdConflict(String),

    #[error("agent tools plugin already installed: {0}")]
    AgentToolsPluginAlreadyInstalled(String),

    #[error("agent recovery plugin already installed: {0}")]
    AgentRecoveryPluginAlreadyInstalled(String),

    #[error("bundle '{bundle_id}' includes unsupported contribution in wiring: {kind}")]
    BundleUnsupportedContribution { bundle_id: String, kind: String },

    #[error("bundle '{bundle_id}' tool id already registered: {id}")]
    BundleToolIdConflict { bundle_id: String, id: String },

    #[error("bundle '{bundle_id}' plugin id mismatch: key={key} plugin.id()={plugin_id}")]
    BundlePluginIdMismatch {
        bundle_id: String,
        key: String,
        plugin_id: String,
    },
}

#[derive(Debug, thiserror::Error)]
pub enum AgentOsBuildError {
    #[error(transparent)]
    Agents(#[from] AgentRegistryError),

    #[error(transparent)]
    Bundle(#[from] BundleComposeError),

    #[error(transparent)]
    Tools(#[from] ToolRegistryError),

    #[error(transparent)]
    Plugins(#[from] PluginRegistryError),

    #[error(transparent)]
    Providers(#[from] ProviderRegistryError),

    #[error(transparent)]
    Models(#[from] ModelRegistryError),

    #[error(transparent)]
    Skills(#[from] SkillError),

    #[error(transparent)]
    SkillRegistry(#[from] SkillRegistryError),

    #[error(transparent)]
    SkillRegistryManager(#[from] SkillRegistryManagerError),

    #[error("agent {agent_id} references an empty plugin id")]
    AgentEmptyPluginRef { agent_id: String },

    #[error("agent {agent_id} references reserved plugin id: {plugin_id}")]
    AgentReservedPluginId { agent_id: String, plugin_id: String },

    #[error("agent {agent_id} references unknown plugin id: {plugin_id}")]
    AgentPluginNotFound { agent_id: String, plugin_id: String },

    #[error("agent {agent_id} has duplicate plugin reference: {plugin_id}")]
    AgentDuplicatePluginRef { agent_id: String, plugin_id: String },

    #[error("models configured but no ProviderRegistry configured")]
    ProvidersNotConfigured,

    #[error("provider not found: {provider_id} (for model id: {model_id})")]
    ProviderNotFound {
        provider_id: String,
        model_id: String,
    },

    #[error("skills enabled but no skills configured")]
    SkillsNotConfigured,
}

#[derive(Debug, thiserror::Error)]
pub enum AgentOsResolveError {
    #[error("agent not found: {0}")]
    AgentNotFound(String),

    #[error("model not found: {0}")]
    ModelNotFound(String),

    #[error("provider not found: {provider_id} (for model id: {model_id})")]
    ProviderNotFound {
        provider_id: String,
        model_id: String,
    },

    #[error(transparent)]
    Wiring(#[from] AgentOsWiringError),
}

#[derive(Debug, thiserror::Error)]
pub enum AgentOsRunError {
    #[error(transparent)]
    Resolve(#[from] AgentOsResolveError),

    #[error(transparent)]
    Loop(#[from] AgentLoopError),

    #[error("agent state store error: {0}")]
    AgentStateStore(#[from] AgentStateStoreError),

    #[error("agent state store not configured")]
    AgentStateStoreNotConfigured,
}

/// Per-run extensions that participate in the resolve/wiring process.
///
/// These are composited into the agent's registries at resolve time,
/// not patched on after assembly.
#[derive(Clone, Default)]
pub struct RunScope {
    /// Additional plugins appended after agent-default plugins.
    pub plugins: Vec<Arc<dyn AgentPlugin>>,
    /// Additional tool registries composited after base tools + system bundles,
    /// but outside `allowed_tools`/`excluded_tools` filtering (overlay semantics).
    pub tool_registries: Vec<Arc<dyn ToolRegistry>>,
}

impl RunScope {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_plugin(mut self, plugin: Arc<dyn AgentPlugin>) -> Self {
        self.plugins.push(plugin);
        self
    }

    pub fn with_plugins(mut self, plugins: Vec<Arc<dyn AgentPlugin>>) -> Self {
        self.plugins.extend(plugins);
        self
    }

    pub fn with_tool_registry(mut self, registry: Arc<dyn ToolRegistry>) -> Self {
        self.tool_registries.push(registry);
        self
    }

    pub fn is_empty(&self) -> bool {
        self.plugins.is_empty() && self.tool_registries.is_empty()
    }
}

/// Result of [`AgentOs::run_stream`]: an event stream plus metadata.
///
/// Checkpoint persistence is handled internally in stream order — callers only
/// consume the event stream and use the IDs for protocol encoding.
///
/// The final thread is **not** exposed here; storage is updated incrementally
/// via `AgentChangeSet` appends.
pub struct RunStream {
    /// Resolved thread ID (may have been auto-generated).
    pub thread_id: String,
    /// Resolved run ID (may have been auto-generated).
    pub run_id: String,
    /// The agent event stream.
    pub events: Pin<Box<dyn Stream<Item = AgentEvent> + Send>>,
}

/// Fully prepared run payload ready for execution.
///
/// This separates request preprocessing from stream execution so preprocessing
/// can be unit-tested deterministically.
pub struct PreparedRun {
    /// Resolved thread ID (may have been auto-generated).
    pub thread_id: String,
    /// Resolved run ID (may have been auto-generated).
    pub run_id: String,
    config: AgentConfig,
    thread: Thread,
    run_ctx: RunServices,
}

impl PreparedRun {
    /// Attach a cooperative cancellation token for this prepared run.
    ///
    /// This keeps loop cancellation wiring outside protocol/UI layers:
    /// transport code can own token lifecycle and inject it before execution.
    #[must_use]
    pub fn with_cancellation_token(
        mut self,
        token: crate::runtime::loop_runner::RunCancellationToken,
    ) -> Self {
        self.run_ctx.cancellation_token = Some(token);
        self
    }
}

#[derive(Clone)]
pub struct AgentOs {
    default_client: Client,
    agents: Arc<dyn AgentRegistry>,
    base_tools: Arc<dyn ToolRegistry>,
    plugins: Arc<dyn PluginRegistry>,
    providers: Arc<dyn ProviderRegistry>,
    models: Arc<dyn ModelRegistry>,
    skills_registry: Option<Arc<dyn SkillRegistry>>,
    skills_config: SkillsConfig,
    agent_runs: Arc<AgentRunManager>,
    agent_tools: AgentToolsConfig,
    agent_state_store: Option<Arc<dyn AgentStateStore>>,
}

#[derive(Clone)]
pub struct AgentOsBuilder {
    client: Option<Client>,
    bundles: Vec<Arc<dyn RegistryBundle>>,
    agents: HashMap<String, AgentDefinition>,
    agent_registries: Vec<Arc<dyn AgentRegistry>>,
    base_tools: HashMap<String, Arc<dyn Tool>>,
    base_tool_registries: Vec<Arc<dyn ToolRegistry>>,
    plugins: HashMap<String, Arc<dyn AgentPlugin>>,
    plugin_registries: Vec<Arc<dyn PluginRegistry>>,
    providers: HashMap<String, Client>,
    provider_registries: Vec<Arc<dyn ProviderRegistry>>,
    models: HashMap<String, ModelDefinition>,
    model_registries: Vec<Arc<dyn ModelRegistry>>,
    skills: Vec<Arc<dyn Skill>>,
    skill_registries: Vec<Arc<dyn SkillRegistry>>,
    skills_refresh_interval: Option<Duration>,
    skills_config: SkillsConfig,
    agent_tools: AgentToolsConfig,
    agent_state_store: Option<Arc<dyn AgentStateStore>>,
}
