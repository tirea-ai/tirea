use std::collections::HashMap;
use std::pin::Pin;
use std::sync::Arc;

use futures::Stream;
use genai::Client;

use crate::plugin::AgentPlugin;
use crate::r#loop::{
    run_loop, run_loop_stream, run_loop_stream_with_checkpoints, run_loop_stream_with_thread,
    RunContext, StreamWithCheckpoints, StreamWithThread,
};
use crate::skills::{
    SkillDiscoveryPlugin, SkillPlugin, SkillRegistry, SkillRuntimePlugin, SkillSubsystem,
    SkillSubsystemError,
};
use crate::storage::{
    CheckpointReason, StorageError, ThreadDelta, ThreadHead, ThreadStore,
};
use crate::tool_filter::{
    set_runtime_filter_if_absent, RUNTIME_ALLOWED_AGENTS_KEY, RUNTIME_ALLOWED_SKILLS_KEY,
    RUNTIME_ALLOWED_TOOLS_KEY, RUNTIME_EXCLUDED_AGENTS_KEY, RUNTIME_EXCLUDED_SKILLS_KEY,
    RUNTIME_EXCLUDED_TOOLS_KEY,
};
use crate::traits::tool::Tool;
use crate::types::Message;
use crate::{AgentConfig, AgentDefinition, AgentEvent, AgentLoopError, Thread};

pub(crate) mod agent_tools;
mod registry;

use agent_tools::{
    AgentRecoveryPlugin, AgentRunManager, AgentRunTool, AgentStopTool, AgentToolsPlugin,
    RUNTIME_CALLER_AGENT_ID_KEY,
};
pub use registry::{
    AgentRegistry, AgentRegistryError, CompositeAgentRegistry, CompositeModelRegistry,
    CompositePluginRegistry, CompositeProviderRegistry, CompositeToolRegistry,
    InMemoryAgentRegistry, InMemoryModelRegistry, InMemoryPluginRegistry, InMemoryProviderRegistry,
    InMemoryToolRegistry, ModelDefinition, ModelRegistry, ModelRegistryError, PluginRegistry,
    PluginRegistryError, ProviderRegistry, ProviderRegistryError, ToolRegistry, ToolRegistryError,
};

type ResolvedAgentWiring = (Client, AgentConfig, HashMap<String, Arc<dyn Tool>>, Thread);

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
            // Skills are opt-in. If enabled, the caller must provide a SkillRegistry.
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

    #[error("skills enabled but no SkillRegistry configured")]
    SkillsNotConfigured,

    #[error("agent tool id already registered: {0}")]
    AgentToolIdConflict(String),

    #[error("agent tools plugin already installed: {0}")]
    AgentToolsPluginAlreadyInstalled(String),

    #[error("agent recovery plugin already installed: {0}")]
    AgentRecoveryPluginAlreadyInstalled(String),
}

#[derive(Debug, thiserror::Error)]
pub enum AgentOsBuildError {
    #[error(transparent)]
    Agents(#[from] AgentRegistryError),

    #[error(transparent)]
    Tools(#[from] ToolRegistryError),

    #[error(transparent)]
    Plugins(#[from] PluginRegistryError),

    #[error(transparent)]
    Providers(#[from] ProviderRegistryError),

    #[error(transparent)]
    Models(#[from] ModelRegistryError),

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

    #[error("skills enabled but no SkillRegistry configured")]
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

    #[error("storage error: {0}")]
    Storage(#[from] StorageError),

    #[error("storage not configured")]
    StorageNotConfigured,
}

/// Request to run an agent. This is the unified entry point for all protocols
/// (AI SDK, AG-UI, etc.) — the transport layer converts protocol-specific
/// requests into this internal format.
#[derive(Debug, Clone)]
pub struct RunRequest {
    pub agent_id: String,
    /// Thread (conversation) ID. `None` → auto-generate (new conversation).
    pub thread_id: Option<String>,
    /// Run ID. `None` → auto-generate.
    pub run_id: Option<String>,
    /// Resource this thread belongs to (for listing/querying).
    pub resource_id: Option<String>,
    /// Initial state for a new thread. Ignored if the thread already exists.
    pub initial_state: Option<serde_json::Value>,
    /// Messages to append before running (already converted to internal format).
    /// Duplicates (by message ID / tool_call_id) are automatically skipped.
    pub messages: Vec<Message>,
    /// Request-scoped runtime context (user_id, token, parent_run_id, etc.).
    pub runtime: HashMap<String, serde_json::Value>,
}

/// Result of [`AgentOs::run_stream`]: an event stream plus metadata.
///
/// Checkpoint persistence is handled internally — callers only consume the
/// event stream and use the IDs for protocol encoding.
pub struct RunStream {
    /// Resolved thread ID (may have been auto-generated).
    pub thread_id: String,
    /// Resolved run ID (may have been auto-generated).
    pub run_id: String,
    /// The agent event stream.
    pub events: Pin<Box<dyn Stream<Item = AgentEvent> + Send>>,
    /// Final in-memory thread (for sub-agent response extraction).
    pub final_thread: tokio::sync::oneshot::Receiver<Thread>,
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
    skills: SkillsConfig,
    agent_runs: Arc<AgentRunManager>,
    agent_tools: AgentToolsConfig,
    storage: Option<Arc<dyn ThreadStore>>,
}

#[derive(Clone)]
pub struct AgentOsBuilder {
    client: Option<Client>,
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
    skills_registry: Option<Arc<dyn SkillRegistry>>,
    skills: SkillsConfig,
    agent_tools: AgentToolsConfig,
    storage: Option<Arc<dyn ThreadStore>>,
}

impl std::fmt::Debug for AgentOs {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AgentOs")
            .field("default_client", &"[genai::Client]")
            .field("agents", &self.agents.len())
            .field("base_tools", &self.base_tools.len())
            .field("plugins", &self.plugins.len())
            .field("providers", &self.providers.len())
            .field("models", &self.models.len())
            .field("skills_registry", &self.skills_registry.is_some())
            .field("skills", &self.skills)
            .field("agent_tools", &self.agent_tools)
            .field("storage", &self.storage.is_some())
            .finish()
    }
}

impl std::fmt::Debug for AgentOsBuilder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AgentOsBuilder")
            .field("client", &self.client.is_some())
            .field("agents", &self.agents.len())
            .field("base_tools", &self.base_tools.len())
            .field("plugins", &self.plugins.len())
            .field("providers", &self.providers.len())
            .field("models", &self.models.len())
            .field("skills_registry", &self.skills_registry.is_some())
            .field("skills", &self.skills)
            .field("agent_tools", &self.agent_tools)
            .field("storage", &self.storage.is_some())
            .finish()
    }
}

impl AgentOsBuilder {
    pub fn new() -> Self {
        Self {
            client: None,
            agents: HashMap::new(),
            agent_registries: Vec::new(),
            base_tools: HashMap::new(),
            base_tool_registries: Vec::new(),
            plugins: HashMap::new(),
            plugin_registries: Vec::new(),
            providers: HashMap::new(),
            provider_registries: Vec::new(),
            models: HashMap::new(),
            model_registries: Vec::new(),
            skills_registry: None,
            skills: SkillsConfig::default(),
            agent_tools: AgentToolsConfig::default(),
            storage: None,
        }
    }

    pub fn with_client(mut self, client: Client) -> Self {
        self.client = Some(client);
        self
    }

    pub fn with_agent(mut self, agent_id: impl Into<String>, def: AgentDefinition) -> Self {
        let agent_id = agent_id.into();
        let mut def = def;
        // The registry key is the canonical id to avoid mismatches.
        def.id = agent_id.clone();
        self.agents.insert(agent_id, def);
        self
    }

    pub fn with_agent_registry(mut self, registry: Arc<dyn AgentRegistry>) -> Self {
        self.agent_registries.push(registry);
        self
    }

    pub fn with_tools(mut self, tools: HashMap<String, Arc<dyn Tool>>) -> Self {
        self.base_tools = tools;
        self
    }

    pub fn with_tool_registry(mut self, registry: Arc<dyn ToolRegistry>) -> Self {
        self.base_tool_registries.push(registry);
        self
    }

    pub fn with_registered_plugin(
        mut self,
        plugin_id: impl Into<String>,
        plugin: Arc<dyn AgentPlugin>,
    ) -> Self {
        self.plugins.insert(plugin_id.into(), plugin);
        self
    }

    pub fn with_plugin_registry(mut self, registry: Arc<dyn PluginRegistry>) -> Self {
        self.plugin_registries.push(registry);
        self
    }

    pub fn with_provider(mut self, provider_id: impl Into<String>, client: Client) -> Self {
        self.providers.insert(provider_id.into(), client);
        self
    }

    pub fn with_provider_registry(mut self, registry: Arc<dyn ProviderRegistry>) -> Self {
        self.provider_registries.push(registry);
        self
    }

    pub fn with_model(mut self, model_id: impl Into<String>, def: ModelDefinition) -> Self {
        self.models.insert(model_id.into(), def);
        self
    }

    pub fn with_models(mut self, defs: HashMap<String, ModelDefinition>) -> Self {
        self.models = defs;
        self
    }

    pub fn with_model_registry(mut self, registry: Arc<dyn ModelRegistry>) -> Self {
        self.model_registries.push(registry);
        self
    }

    pub fn with_skills_registry(mut self, registry: Arc<dyn SkillRegistry>) -> Self {
        self.skills_registry = Some(registry);
        self
    }

    pub fn with_skills_config(mut self, cfg: SkillsConfig) -> Self {
        self.skills = cfg;
        self
    }

    pub fn with_agent_tools_config(mut self, cfg: AgentToolsConfig) -> Self {
        self.agent_tools = cfg;
        self
    }

    pub fn with_storage(mut self, storage: Arc<dyn ThreadStore>) -> Self {
        self.storage = Some(storage);
        self
    }

    pub fn build(self) -> Result<AgentOs, AgentOsBuildError> {
        let AgentOsBuilder {
            client,
            agents: agents_defs,
            agent_registries,
            base_tools: base_tools_defs,
            base_tool_registries,
            plugins: plugin_defs,
            plugin_registries,
            providers: provider_defs,
            provider_registries,
            models: model_defs,
            model_registries,
            skills_registry,
            skills,
            agent_tools,
            storage,
        } = self;

        if skills.mode != SkillsMode::Disabled && skills_registry.is_none() {
            return Err(AgentOsBuildError::SkillsNotConfigured);
        }

        let skills_registry: Option<Arc<dyn SkillRegistry>> = skills_registry;

        let mut base_tools = InMemoryToolRegistry::new();
        base_tools.extend_named(base_tools_defs)?;

        let base_tools: Arc<dyn ToolRegistry> = match base_tool_registries.len() {
            0 => Arc::new(base_tools),
            _ => {
                let mut regs: Vec<Arc<dyn ToolRegistry>> = Vec::new();
                if !base_tools.is_empty() {
                    regs.push(Arc::new(base_tools));
                }
                regs.extend(base_tool_registries);
                if regs.len() == 1 {
                    regs.pop().unwrap()
                } else {
                    Arc::new(CompositeToolRegistry::try_new(regs)?)
                }
            }
        };

        let mut plugins = InMemoryPluginRegistry::new();
        plugins.extend_named(plugin_defs)?;

        let plugins: Arc<dyn PluginRegistry> = match plugin_registries.len() {
            0 => Arc::new(plugins),
            _ => {
                let mut regs: Vec<Arc<dyn PluginRegistry>> = Vec::new();
                if !plugins.is_empty() {
                    regs.push(Arc::new(plugins));
                }
                regs.extend(plugin_registries);
                if regs.len() == 1 {
                    regs.pop().unwrap()
                } else {
                    Arc::new(CompositePluginRegistry::try_new(regs)?)
                }
            }
        };

        // Fail-fast for builder-provided agents (external registries may be dynamic).
        {
            let reserved = AgentOs::reserved_plugin_ids();
            for (agent_id, def) in &agents_defs {
                let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
                for id in def.policy_ids.iter().chain(def.plugin_ids.iter()) {
                    let id = id.trim();
                    if id.is_empty() {
                        return Err(AgentOsBuildError::AgentEmptyPluginRef {
                            agent_id: agent_id.clone(),
                        });
                    }
                    if reserved.contains(&id) {
                        return Err(AgentOsBuildError::AgentReservedPluginId {
                            agent_id: agent_id.clone(),
                            plugin_id: id.to_string(),
                        });
                    }
                    if !seen.insert(id.to_string()) {
                        return Err(AgentOsBuildError::AgentDuplicatePluginRef {
                            agent_id: agent_id.clone(),
                            plugin_id: id.to_string(),
                        });
                    }
                    if plugins.get(id).is_none() {
                        return Err(AgentOsBuildError::AgentPluginNotFound {
                            agent_id: agent_id.clone(),
                            plugin_id: id.to_string(),
                        });
                    }
                }
            }
        }

        let mut providers = InMemoryProviderRegistry::new();
        providers.extend(provider_defs)?;

        let providers: Arc<dyn ProviderRegistry> = match provider_registries.len() {
            0 => Arc::new(providers),
            _ => {
                let mut regs: Vec<Arc<dyn ProviderRegistry>> = Vec::new();
                if !providers.is_empty() {
                    regs.push(Arc::new(providers));
                }
                regs.extend(provider_registries);
                if regs.len() == 1 {
                    regs.pop().unwrap()
                } else {
                    Arc::new(CompositeProviderRegistry::try_new(regs)?)
                }
            }
        };

        let mut models = InMemoryModelRegistry::new();
        models.extend(model_defs.clone())?;

        let models: Arc<dyn ModelRegistry> = match model_registries.len() {
            0 => Arc::new(models),
            _ => {
                let mut regs: Vec<Arc<dyn ModelRegistry>> = Vec::new();
                if !models.is_empty() {
                    regs.push(Arc::new(models));
                }
                regs.extend(model_registries);
                if regs.len() == 1 {
                    regs.pop().unwrap()
                } else {
                    Arc::new(CompositeModelRegistry::try_new(regs)?)
                }
            }
        };

        if !models.is_empty() && providers.is_empty() {
            return Err(AgentOsBuildError::ProvidersNotConfigured);
        }

        for (model_id, def) in models.snapshot() {
            if providers.get(&def.provider).is_none() {
                return Err(AgentOsBuildError::ProviderNotFound {
                    provider_id: def.provider,
                    model_id,
                });
            }
        }

        let mut agents = InMemoryAgentRegistry::new();
        agents.extend_upsert(agents_defs);

        let agents: Arc<dyn AgentRegistry> = match agent_registries.len() {
            0 => Arc::new(agents),
            _ => {
                let mut regs: Vec<Arc<dyn AgentRegistry>> = Vec::new();
                if !agents.is_empty() {
                    regs.push(Arc::new(agents));
                }
                regs.extend(agent_registries);
                if regs.len() == 1 {
                    regs.pop().unwrap()
                } else {
                    Arc::new(CompositeAgentRegistry::try_new(regs)?)
                }
            }
        };

        Ok(AgentOs {
            default_client: client.unwrap_or_default(),
            agents,
            base_tools,
            plugins,
            providers,
            models,
            skills_registry,
            skills,
            agent_runs: Arc::new(AgentRunManager::new()),
            agent_tools,
            storage,
        })
    }
}

impl Default for AgentOsBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl AgentOs {
    pub fn builder() -> AgentOsBuilder {
        AgentOsBuilder::new()
    }

    pub fn client(&self) -> Client {
        self.default_client.clone()
    }

    pub fn skill_registry(&self) -> Option<Arc<dyn SkillRegistry>> {
        self.skills_registry.clone()
    }

    pub(crate) fn agents_registry(&self) -> Arc<dyn AgentRegistry> {
        self.agents.clone()
    }

    pub fn agent(&self, agent_id: &str) -> Option<AgentDefinition> {
        self.agents.get(agent_id)
    }

    pub fn tools(&self) -> HashMap<String, Arc<dyn Tool>> {
        self.base_tools.snapshot()
    }

    fn reserved_plugin_ids() -> &'static [&'static str] {
        &[
            "skills",
            "skills_discovery",
            "skills_runtime",
            "agent_tools",
            "agent_recovery",
        ]
    }

    fn reserved_skills_plugin_ids() -> &'static [&'static str] {
        &["skills", "skills_discovery", "skills_runtime"]
    }

    fn resolve_plugin_id_list(
        &self,
        plugin_ids: &[String],
    ) -> Result<Vec<Arc<dyn AgentPlugin>>, AgentOsWiringError> {
        let reserved = Self::reserved_plugin_ids();
        let mut out: Vec<Arc<dyn AgentPlugin>> = Vec::new();
        for id in plugin_ids {
            let id = id.trim();
            if reserved.contains(&id) {
                return Err(AgentOsWiringError::ReservedPluginId(id.to_string()));
            }
            let p = self
                .plugins
                .get(id)
                .ok_or_else(|| AgentOsWiringError::PluginNotFound(id.to_string()))?;
            out.push(p);
        }
        Ok(out)
    }

    fn ensure_unique_plugin_ids(
        plugins: &[Arc<dyn AgentPlugin>],
    ) -> Result<(), AgentOsWiringError> {
        // Fail-fast: plugins are keyed by `AgentPlugin::id()`. Duplicates are almost always a bug.
        let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
        for p in plugins {
            let id = p.id().to_string();
            if !seen.insert(id.clone()) {
                return Err(AgentOsWiringError::PluginAlreadyInstalled(id));
            }
        }
        Ok(())
    }

    fn assemble_plugin_chain(
        system_plugins: Vec<Arc<dyn AgentPlugin>>,
        policy_plugins: Vec<Arc<dyn AgentPlugin>>,
        other_plugins: Vec<Arc<dyn AgentPlugin>>,
        explicit_plugins: Vec<Arc<dyn AgentPlugin>>,
    ) -> Result<Vec<Arc<dyn AgentPlugin>>, AgentOsWiringError> {
        let mut plugins = Vec::new();
        plugins.extend(system_plugins);
        plugins.extend(policy_plugins);
        plugins.extend(other_plugins);
        plugins.extend(explicit_plugins);
        Self::ensure_unique_plugin_ids(&plugins)?;
        Ok(plugins)
    }

    fn ensure_skills_plugin_not_installed(
        plugins: &[Arc<dyn AgentPlugin>],
    ) -> Result<(), AgentOsWiringError> {
        let reserved = Self::reserved_skills_plugin_ids();
        if let Some(existing) = plugins
            .iter()
            .map(|p| p.id())
            .find(|id| reserved.contains(id))
        {
            return Err(AgentOsWiringError::SkillsPluginAlreadyInstalled(
                existing.to_string(),
            ));
        }
        Ok(())
    }

    fn ensure_agent_tools_plugin_not_installed(
        plugins: &[Arc<dyn AgentPlugin>],
    ) -> Result<(), AgentOsWiringError> {
        for existing in plugins.iter().map(|p| p.id()) {
            if existing == "agent_tools" {
                return Err(AgentOsWiringError::AgentToolsPluginAlreadyInstalled(
                    existing.to_string(),
                ));
            }
            if existing == "agent_recovery" {
                return Err(AgentOsWiringError::AgentRecoveryPluginAlreadyInstalled(
                    existing.to_string(),
                ));
            }
        }
        Ok(())
    }

    fn build_skills_plugins(&self, reg: Arc<dyn SkillRegistry>) -> Vec<Arc<dyn AgentPlugin>> {
        match self.skills.mode {
            SkillsMode::Disabled => Vec::new(),
            SkillsMode::DiscoveryAndRuntime => {
                let discovery = SkillDiscoveryPlugin::new(reg).with_limits(
                    self.skills.discovery_max_entries,
                    self.skills.discovery_max_chars,
                );
                vec![SkillPlugin::new(discovery).boxed()]
            }
            SkillsMode::DiscoveryOnly => {
                let discovery = SkillDiscoveryPlugin::new(reg).with_limits(
                    self.skills.discovery_max_entries,
                    self.skills.discovery_max_chars,
                );
                vec![Arc::new(discovery)]
            }
            SkillsMode::RuntimeOnly => vec![Arc::new(SkillRuntimePlugin::new())],
        }
    }

    fn wire_skills_plugins_and_tools(
        &self,
        explicit_plugins: &[Arc<dyn AgentPlugin>],
        tools: &mut HashMap<String, Arc<dyn Tool>>,
    ) -> Result<Vec<Arc<dyn AgentPlugin>>, AgentOsWiringError> {
        if self.skills.mode == SkillsMode::Disabled {
            return Ok(Vec::new());
        }

        Self::ensure_skills_plugin_not_installed(explicit_plugins)?;
        let reg = self
            .skills_registry
            .clone()
            .ok_or(AgentOsWiringError::SkillsNotConfigured)?;

        let skills = SkillSubsystem::new(reg.clone());
        skills.extend_tools(tools).map_err(|e| match e {
            SkillSubsystemError::ToolIdConflict(id) => AgentOsWiringError::SkillsToolIdConflict(id),
        })?;

        Ok(self.build_skills_plugins(reg))
    }

    fn wire_agent_tools_plugins_and_tools(
        &self,
        explicit_plugins: &[Arc<dyn AgentPlugin>],
        tools: &mut HashMap<String, Arc<dyn Tool>>,
    ) -> Result<Vec<Arc<dyn AgentPlugin>>, AgentOsWiringError> {
        Self::ensure_agent_tools_plugin_not_installed(explicit_plugins)?;

        let run_tool: Arc<dyn Tool> =
            Arc::new(AgentRunTool::new(self.clone(), self.agent_runs.clone()));
        let run_tool_id = run_tool.descriptor().id.clone();
        if tools.contains_key(&run_tool_id) {
            return Err(AgentOsWiringError::AgentToolIdConflict(run_tool_id));
        }
        tools.insert(run_tool.descriptor().id.clone(), run_tool);

        let stop_tool: Arc<dyn Tool> = Arc::new(AgentStopTool::new(self.agent_runs.clone()));
        let stop_tool_id = stop_tool.descriptor().id.clone();
        if tools.contains_key(&stop_tool_id) {
            return Err(AgentOsWiringError::AgentToolIdConflict(stop_tool_id));
        }
        tools.insert(stop_tool.descriptor().id.clone(), stop_tool);

        let tools_plugin = AgentToolsPlugin::new(self.agents.clone(), self.agent_runs.clone())
            .with_limits(
                self.agent_tools.discovery_max_entries,
                self.agent_tools.discovery_max_chars,
            );
        let recovery_plugin = AgentRecoveryPlugin::new(self.agent_runs.clone());
        Ok(vec![Arc::new(tools_plugin), Arc::new(recovery_plugin)])
    }

    pub fn wire_plugins_into(
        &self,
        mut config: AgentConfig,
    ) -> Result<AgentConfig, AgentOsWiringError> {
        if config.policy_ids.is_empty() && config.plugin_ids.is_empty() {
            return Ok(config);
        }

        let policy_plugins = self.resolve_plugin_id_list(&config.policy_ids)?;
        let other_plugins = self.resolve_plugin_id_list(&config.plugin_ids)?;
        config.plugins =
            Self::assemble_plugin_chain(Vec::new(), policy_plugins, other_plugins, config.plugins)?;
        Ok(config)
    }

    pub fn wire_into(
        &self,
        mut config: AgentConfig,
        tools: &mut HashMap<String, Arc<dyn Tool>>,
    ) -> Result<AgentConfig, AgentOsWiringError> {
        // Explicit wiring order:
        // system(skills, agent_tools, agent_recovery) -> policies -> plugins -> explicit.
        let explicit_plugins = std::mem::take(&mut config.plugins);
        let policy_plugins = self.resolve_plugin_id_list(&config.policy_ids)?;
        let other_plugins = self.resolve_plugin_id_list(&config.plugin_ids)?;

        let mut system_plugins = self.wire_skills_plugins_and_tools(&explicit_plugins, tools)?;
        system_plugins.extend(self.wire_agent_tools_plugins_and_tools(&explicit_plugins, tools)?);
        config.plugins = Self::assemble_plugin_chain(
            system_plugins,
            policy_plugins,
            other_plugins,
            explicit_plugins,
        )?;
        Ok(config)
    }

    fn resolve_model(&self, cfg: &mut AgentConfig) -> Result<Client, AgentOsResolveError> {
        if self.models.is_empty() {
            return Ok(self.default_client.clone());
        }

        let Some(def) = self.models.get(&cfg.model) else {
            return Err(AgentOsResolveError::ModelNotFound(cfg.model.clone()));
        };

        let Some(client) = self.providers.get(&def.provider) else {
            return Err(AgentOsResolveError::ProviderNotFound {
                provider_id: def.provider.clone(),
                model_id: cfg.model.clone(),
            });
        };

        cfg.model = def.model;
        if let Some(opts) = def.chat_options {
            cfg.chat_options = Some(opts);
        }
        Ok(client)
    }

    pub fn wire_skills_into(
        &self,
        mut config: AgentConfig,
        tools: &mut HashMap<String, Arc<dyn Tool>>,
    ) -> Result<AgentConfig, AgentOsWiringError> {
        let explicit_plugins = std::mem::take(&mut config.plugins);
        let skills_plugins = self.wire_skills_plugins_and_tools(&explicit_plugins, tools)?;
        config.plugins =
            Self::assemble_plugin_chain(skills_plugins, Vec::new(), Vec::new(), explicit_plugins)?;
        Ok(config)
    }

    /// Check whether an agent with the given ID is registered.
    pub fn validate_agent(&self, agent_id: &str) -> Result<(), AgentOsResolveError> {
        if self.agents.get(agent_id).is_some() {
            Ok(())
        } else {
            Err(AgentOsResolveError::AgentNotFound(agent_id.to_string()))
        }
    }

    pub fn resolve(
        &self,
        agent_id: &str,
        mut thread: Thread,
    ) -> Result<ResolvedAgentWiring, AgentOsResolveError> {
        let def = self
            .agents
            .get(agent_id)
            .ok_or_else(|| AgentOsResolveError::AgentNotFound(agent_id.to_string()))?;

        if thread.runtime.value(RUNTIME_CALLER_AGENT_ID_KEY).is_none() {
            let _ = thread
                .runtime
                .set(RUNTIME_CALLER_AGENT_ID_KEY, agent_id.to_string());
        }
        let _ = set_runtime_filter_if_absent(
            &mut thread.runtime,
            RUNTIME_ALLOWED_TOOLS_KEY,
            def.allowed_tools.as_deref(),
        );
        let _ = set_runtime_filter_if_absent(
            &mut thread.runtime,
            RUNTIME_EXCLUDED_TOOLS_KEY,
            def.excluded_tools.as_deref(),
        );
        let _ = set_runtime_filter_if_absent(
            &mut thread.runtime,
            RUNTIME_ALLOWED_SKILLS_KEY,
            def.allowed_skills.as_deref(),
        );
        let _ = set_runtime_filter_if_absent(
            &mut thread.runtime,
            RUNTIME_EXCLUDED_SKILLS_KEY,
            def.excluded_skills.as_deref(),
        );
        let _ = set_runtime_filter_if_absent(
            &mut thread.runtime,
            RUNTIME_ALLOWED_AGENTS_KEY,
            def.allowed_agents.as_deref(),
        );
        let _ = set_runtime_filter_if_absent(
            &mut thread.runtime,
            RUNTIME_EXCLUDED_AGENTS_KEY,
            def.excluded_agents.as_deref(),
        );

        let mut tools = self.base_tools.snapshot();
        let mut cfg = self.wire_into(def, &mut tools)?;
        let client = self.resolve_model(&mut cfg)?;
        Ok((client, cfg, tools, thread))
    }

    pub fn storage(&self) -> Option<&Arc<dyn ThreadStore>> {
        self.storage.as_ref()
    }

    fn require_storage(&self) -> Result<&Arc<dyn ThreadStore>, AgentOsRunError> {
        self.storage
            .as_ref()
            .ok_or(AgentOsRunError::StorageNotConfigured)
    }

    fn generate_id() -> String {
        uuid::Uuid::now_v7().simple().to_string()
    }

    /// Load a thread from storage. Returns the thread and its version.
    /// If the thread does not exist, returns `None`.
    pub async fn load_thread(&self, id: &str) -> Result<Option<ThreadHead>, AgentOsRunError> {
        let storage = self.require_storage()?;
        Ok(storage.load(id).await?)
    }

    /// Run an agent from a [`RunRequest`].
    ///
    /// This is the primary entry point for all protocols. It handles:
    /// 1. Thread loading/creation from storage
    /// 2. Message deduplication and appending
    /// 3. Persisting pre-run state
    /// 4. Agent resolution and execution
    /// 5. Background checkpoint persistence
    pub async fn run_stream(&self, request: RunRequest) -> Result<RunStream, AgentOsRunError> {
        let storage = self.require_storage()?;

        // 0. Validate agent exists (fail fast before creating thread)
        self.validate_agent(&request.agent_id)?;

        let thread_id = request.thread_id.unwrap_or_else(Self::generate_id);
        let run_id = request.run_id.unwrap_or_else(Self::generate_id);

        // 1. Load or create thread
        let mut thread = match storage.load(&thread_id).await? {
            Some(head) => head.thread,
            None => {
                let thread = if let Some(state) = request.initial_state {
                    Thread::with_initial_state(thread_id.clone(), state)
                } else {
                    Thread::new(thread_id.clone())
                };
                storage.create(&thread).await?;
                thread
            }
        };

        // 2. Set resource_id on thread if provided
        if let Some(ref resource_id) = request.resource_id {
            thread.resource_id = Some(resource_id.clone());
        }

        // 3. Apply request-scoped runtime context
        for (key, value) in &request.runtime {
            let _ = thread.runtime.set(key, value.clone());
        }

        // 4. Set run_id on thread runtime
        let _ = thread.runtime.set("run_id", run_id.clone());

        // 5. Deduplicate and append inbound messages
        let deduped = Self::dedup_messages(&thread, request.messages);
        if !deduped.is_empty() {
            thread = thread.with_messages(deduped);
        }

        // 6. Persist pending changes (user messages, etc.)
        let pending = thread.take_pending();
        if !pending.is_empty() {
            let delta = ThreadDelta {
                run_id: run_id.clone(),
                parent_run_id: request
                    .runtime
                    .get("parent_run_id")
                    .and_then(|v| v.as_str())
                    .map(String::from),
                reason: CheckpointReason::UserMessage,
                messages: pending.messages,
                patches: pending.patches,
                snapshot: None,
            };
            storage.append(&thread_id, &delta).await?;
        }

        // 7. Resolve agent wiring and run
        let (client, cfg, tools, thread) = self.resolve(&request.agent_id, thread)?;

        let swc = run_loop_stream_with_checkpoints(
            client,
            cfg,
            thread,
            tools,
            RunContext::default(),
        );

        // 8. Spawn background checkpoint persistence
        {
            let storage = storage.clone();
            let thread_id = thread_id.clone();
            let mut checkpoints = swc.checkpoints;
            tokio::spawn(async move {
                while let Some(delta) = checkpoints.recv().await {
                    if let Err(e) = storage.append(&thread_id, &delta).await {
                        tracing::error!(
                            thread_id = %thread_id,
                            error = %e,
                            "failed to persist checkpoint"
                        );
                    }
                }
            });
        }

        Ok(RunStream {
            thread_id,
            run_id,
            events: swc.events,
            final_thread: swc.final_thread,
        })
    }

    /// Deduplicate incoming messages against existing thread messages.
    ///
    /// Skips messages whose ID or tool_call_id already exists in the thread.
    fn dedup_messages(thread: &Thread, incoming: Vec<Message>) -> Vec<Message> {
        use std::collections::HashSet;

        let existing_ids: HashSet<&str> = thread
            .messages
            .iter()
            .filter_map(|m| m.id.as_deref())
            .collect();
        let existing_tool_call_ids: HashSet<&str> = thread
            .messages
            .iter()
            .filter_map(|m| m.tool_call_id.as_deref())
            .collect();

        incoming
            .into_iter()
            .filter(|m| {
                // Dedup tool messages by tool_call_id
                if let Some(ref tc_id) = m.tool_call_id {
                    if existing_tool_call_ids.contains(tc_id.as_str()) {
                        return false;
                    }
                }
                // Dedup by message id
                if let Some(ref id) = m.id {
                    if existing_ids.contains(id.as_str()) {
                        return false;
                    }
                }
                true
            })
            .collect()
    }

    // --- Legacy methods (used by SubAgentTool and tests) ---

    pub async fn run_blocking(
        &self,
        agent_id: &str,
        thread: Thread,
    ) -> Result<(Thread, String), AgentOsRunError> {
        let (client, cfg, tools, thread) = self.resolve(agent_id, thread)?;
        let (thread, text) = run_loop(&client, &cfg, thread, &tools).await?;
        Ok((thread, text))
    }

    pub fn run_stream_raw(
        &self,
        agent_id: &str,
        thread: Thread,
    ) -> Result<impl futures::Stream<Item = AgentEvent> + Send, AgentOsResolveError> {
        let (client, cfg, tools, thread) = self.resolve(agent_id, thread)?;
        Ok(run_loop_stream(
            client,
            cfg,
            thread,
            tools,
            RunContext::default(),
        ))
    }

    pub fn run_stream_with_context(
        &self,
        agent_id: &str,
        thread: Thread,
        run_ctx: RunContext,
    ) -> Result<impl futures::Stream<Item = AgentEvent> + Send, AgentOsResolveError> {
        let (client, cfg, tools, thread) = self.resolve(agent_id, thread)?;
        Ok(run_loop_stream(client, cfg, thread, tools, run_ctx))
    }

    pub fn run_stream_with_session(
        &self,
        agent_id: &str,
        thread: Thread,
        run_ctx: RunContext,
    ) -> Result<StreamWithThread, AgentOsResolveError> {
        let (client, cfg, tools, thread) = self.resolve(agent_id, thread)?;
        Ok(run_loop_stream_with_thread(
            client, cfg, thread, tools, run_ctx,
        ))
    }

    pub fn run_stream_with_checkpoints(
        &self,
        agent_id: &str,
        thread: Thread,
        run_ctx: RunContext,
    ) -> Result<StreamWithCheckpoints, AgentOsResolveError> {
        let (client, cfg, tools, thread) = self.resolve(agent_id, thread)?;
        Ok(run_loop_stream_with_checkpoints(
            client, cfg, thread, tools, run_ctx,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::phase::{Phase, StepContext};
    use crate::skills::FsSkillRegistry;
    use crate::thread::Thread;
    use crate::traits::tool::ToolDescriptor;
    use crate::traits::tool::{ToolError, ToolResult};
    use async_trait::async_trait;
    use serde_json::json;
    use std::fs;
    use std::path::PathBuf;
    use tempfile::TempDir;

    fn make_skills_root() -> (TempDir, PathBuf) {
        let td = TempDir::new().unwrap();
        let root = td.path().join("skills");
        fs::create_dir_all(root.join("s1")).unwrap();
        fs::write(
            root.join("s1").join("SKILL.md"),
            "---\nname: s1\ndescription: ok\n---\nDo X\n",
        )
        .unwrap();
        (td, root)
    }

    #[tokio::test]
    async fn wire_skills_inserts_tools_and_plugin() {
        let (_td, root) = make_skills_root();
        let os = AgentOs::builder()
            .with_skills_registry(Arc::new(FsSkillRegistry::discover_root(root).unwrap()))
            .with_skills_config(SkillsConfig {
                mode: SkillsMode::DiscoveryAndRuntime,
                ..SkillsConfig::default()
            })
            .build()
            .unwrap();

        let mut tools: HashMap<String, Arc<dyn Tool>> = HashMap::new();
        let cfg = AgentConfig::new("gpt-4o-mini");
        let cfg = os.wire_skills_into(cfg, &mut tools).unwrap();

        assert!(tools.contains_key("skill"));
        assert!(tools.contains_key("load_skill_resource"));
        assert!(tools.contains_key("skill_script"));

        assert_eq!(cfg.plugins.len(), 1);
        assert_eq!(cfg.plugins[0].id(), "skills");

        // Verify injection does not panic and includes catalog.
        let thread = Thread::with_initial_state(
            "s",
            json!({
                "skills": {
                    "active": ["s1"],
                    "instructions": {"s1": "Do X"},
                    "references": {},
                    "scripts": {}
                }
            }),
        );
        let mut step = StepContext::new(&thread, vec![ToolDescriptor::new("t", "t", "t")]);
        cfg.plugins[0]
            .on_phase(Phase::BeforeInference, &mut step)
            .await;
        assert_eq!(step.system_context.len(), 1);
        assert!(step.system_context[0].contains("<available_skills>"));
    }

    #[tokio::test]
    async fn wire_skills_runtime_only_injects_active_skills_without_catalog() {
        let (_td, root) = make_skills_root();
        let os = AgentOs::builder()
            .with_skills_registry(Arc::new(FsSkillRegistry::discover_root(root).unwrap()))
            .with_skills_config(SkillsConfig {
                mode: SkillsMode::RuntimeOnly,
                ..SkillsConfig::default()
            })
            .build()
            .unwrap();

        let mut tools: HashMap<String, Arc<dyn Tool>> = HashMap::new();
        let cfg = AgentConfig::new("gpt-4o-mini");
        let cfg = os.wire_skills_into(cfg, &mut tools).unwrap();

        assert_eq!(cfg.plugins.len(), 1);
        assert_eq!(cfg.plugins[0].id(), "skills_runtime");

        let thread = Thread::with_initial_state(
            "s",
            json!({
                "skills": {
                    "active": ["s1"],
                    "instructions": {"s1": "Do X"},
                    "references": {},
                    "scripts": {}
                }
            }),
        );
        let mut step = StepContext::new(&thread, vec![ToolDescriptor::new("t", "t", "t")]);
        cfg.plugins[0]
            .on_phase(Phase::BeforeInference, &mut step)
            .await;
        assert!(step.system_context.is_empty());
    }

    #[test]
    fn wire_skills_disabled_is_noop() {
        let (_td, root) = make_skills_root();
        let os = AgentOs::builder()
            .with_skills_registry(Arc::new(FsSkillRegistry::discover_root(root).unwrap()))
            .with_skills_config(SkillsConfig {
                mode: SkillsMode::Disabled,
                ..SkillsConfig::default()
            })
            .build()
            .unwrap();

        let mut tools: HashMap<String, Arc<dyn Tool>> = HashMap::new();
        let cfg = AgentConfig::new("gpt-4o-mini");
        let cfg2 = os.wire_skills_into(cfg.clone(), &mut tools).unwrap();

        assert!(tools.is_empty());
        assert_eq!(cfg2.plugins.len(), cfg.plugins.len());
    }

    #[test]
    fn wire_plugins_into_orders_policy_then_plugin_then_explicit() {
        #[derive(Debug)]
        struct LocalPlugin(&'static str);

        #[async_trait]
        impl AgentPlugin for LocalPlugin {
            fn id(&self) -> &str {
                self.0
            }

            async fn on_phase(&self, _phase: Phase, _step: &mut StepContext<'_>) {}
        }

        let os = AgentOs::builder()
            .with_registered_plugin("policy1", Arc::new(LocalPlugin("policy1")))
            .with_registered_plugin("p1", Arc::new(LocalPlugin("p1")))
            .build()
            .unwrap();

        let cfg = AgentConfig::new("gpt-4o-mini")
            .with_policy_id("policy1")
            .with_plugin_id("p1")
            .with_plugin(Arc::new(LocalPlugin("explicit")));

        let wired = os.wire_plugins_into(cfg).unwrap();
        let ids: Vec<&str> = wired.plugins.iter().map(|p| p.id()).collect();
        assert_eq!(ids, vec!["policy1", "p1", "explicit"]);
    }

    #[test]
    fn wire_plugins_into_rejects_duplicate_plugin_ids_after_assembly() {
        #[derive(Debug)]
        struct LocalPlugin(&'static str);

        #[async_trait]
        impl AgentPlugin for LocalPlugin {
            fn id(&self) -> &str {
                self.0
            }

            async fn on_phase(&self, _phase: Phase, _step: &mut StepContext<'_>) {}
        }

        let os = AgentOs::builder()
            .with_registered_plugin("p1", Arc::new(LocalPlugin("p1")))
            .build()
            .unwrap();

        let cfg = AgentConfig::new("gpt-4o-mini")
            .with_plugin_id("p1")
            .with_plugin(Arc::new(LocalPlugin("p1")));

        let err = os.wire_plugins_into(cfg).unwrap_err();
        assert!(matches!(err, AgentOsWiringError::PluginAlreadyInstalled(id) if id == "p1"));
    }

    #[derive(Debug)]
    struct FakeSkillsPlugin;

    #[async_trait::async_trait]
    impl AgentPlugin for FakeSkillsPlugin {
        fn id(&self) -> &str {
            "skills"
        }

        async fn on_phase(&self, _phase: Phase, _step: &mut StepContext<'_>) {}
    }

    #[test]
    fn wire_skills_errors_if_plugin_already_installed() {
        let (_td, root) = make_skills_root();
        let os = AgentOs::builder()
            .with_skills_registry(Arc::new(FsSkillRegistry::discover_root(root).unwrap()))
            .with_skills_config(SkillsConfig {
                mode: SkillsMode::DiscoveryAndRuntime,
                ..SkillsConfig::default()
            })
            .build()
            .unwrap();

        let mut tools: HashMap<String, Arc<dyn Tool>> = HashMap::new();
        let cfg = AgentConfig::new("gpt-4o-mini").with_plugin(Arc::new(FakeSkillsPlugin));

        let err = os.wire_skills_into(cfg, &mut tools).unwrap_err();
        assert!(err.to_string().contains("skills plugin already installed"));
    }

    #[derive(Debug)]
    struct FakeAgentToolsPlugin;

    #[async_trait::async_trait]
    impl AgentPlugin for FakeAgentToolsPlugin {
        fn id(&self) -> &str {
            "agent_tools"
        }

        async fn on_phase(&self, _phase: Phase, _step: &mut StepContext<'_>) {}
    }

    #[test]
    fn resolve_errors_if_agent_tools_plugin_already_installed() {
        let os = AgentOs::builder()
            .with_agent(
                "a1",
                AgentDefinition::new("gpt-4o-mini").with_plugin(Arc::new(FakeAgentToolsPlugin)),
            )
            .build()
            .unwrap();

        let thread = Thread::with_initial_state("s", json!({}));
        let err = os.resolve("a1", thread).err().unwrap();
        assert!(matches!(
            err,
            AgentOsResolveError::Wiring(AgentOsWiringError::AgentToolsPluginAlreadyInstalled(ref id))
            if id == "agent_tools"
        ));
    }

    #[derive(Debug)]
    struct FakeAgentRecoveryPlugin;

    #[async_trait::async_trait]
    impl AgentPlugin for FakeAgentRecoveryPlugin {
        fn id(&self) -> &str {
            "agent_recovery"
        }

        async fn on_phase(&self, _phase: Phase, _step: &mut StepContext<'_>) {}
    }

    #[test]
    fn resolve_errors_if_agent_recovery_plugin_already_installed() {
        let os = AgentOs::builder()
            .with_agent(
                "a1",
                AgentDefinition::new("gpt-4o-mini").with_plugin(Arc::new(FakeAgentRecoveryPlugin)),
            )
            .build()
            .unwrap();

        let thread = Thread::with_initial_state("s", json!({}));
        let err = os.resolve("a1", thread).err().unwrap();
        assert!(matches!(
            err,
            AgentOsResolveError::Wiring(
                AgentOsWiringError::AgentRecoveryPluginAlreadyInstalled(ref id)
            ) if id == "agent_recovery"
        ));
    }

    #[test]
    fn resolve_errors_if_agent_missing() {
        let os = AgentOs::builder().build().unwrap();
        let thread = Thread::with_initial_state("s", json!({}));
        let err = os.resolve("missing", thread).err().unwrap();
        assert!(matches!(err, AgentOsResolveError::AgentNotFound(_)));
    }

    #[tokio::test]
    async fn resolve_wires_skills_and_preserves_base_tools() {
        #[derive(Debug)]
        struct BaseTool;

        #[async_trait::async_trait]
        impl Tool for BaseTool {
            fn descriptor(&self) -> ToolDescriptor {
                ToolDescriptor::new("base_tool", "Base Tool", "Base Tool")
            }

            async fn execute(
                &self,
                _args: serde_json::Value,
                _ctx: &carve_state::Context<'_>,
            ) -> Result<ToolResult, ToolError> {
                Ok(ToolResult::success("base_tool", json!({"ok": true})))
            }
        }

        let (_td, root) = make_skills_root();
        let os = AgentOs::builder()
            .with_skills_registry(Arc::new(FsSkillRegistry::discover_root(root).unwrap()))
            .with_skills_config(SkillsConfig {
                mode: SkillsMode::DiscoveryAndRuntime,
                ..SkillsConfig::default()
            })
            .with_agent("a1", AgentDefinition::new("gpt-4o-mini"))
            .with_tools(HashMap::from([(
                "base_tool".to_string(),
                Arc::new(BaseTool) as Arc<dyn Tool>,
            )]))
            .build()
            .unwrap();

        let thread = Thread::with_initial_state("s", json!({}));
        let (_client, cfg, tools, _thread) = os.resolve("a1", thread).unwrap();

        assert_eq!(cfg.id, "a1");
        assert!(tools.contains_key("base_tool"));
        assert!(tools.contains_key("skill"));
        assert!(tools.contains_key("load_skill_resource"));
        assert!(tools.contains_key("skill_script"));
        assert!(tools.contains_key("agent_run"));
        assert!(tools.contains_key("agent_stop"));
        assert_eq!(cfg.plugins.len(), 3);
        assert_eq!(cfg.plugins[0].id(), "skills");
        assert_eq!(cfg.plugins[1].id(), "agent_tools");
        assert_eq!(cfg.plugins[2].id(), "agent_recovery");
    }

    #[tokio::test]
    async fn run_and_run_stream_work_without_llm_when_skip_inference() {
        #[derive(Debug)]
        struct SkipInferencePlugin;

        #[async_trait::async_trait]
        impl AgentPlugin for SkipInferencePlugin {
            fn id(&self) -> &str {
                "skip_inference"
            }

            async fn on_phase(&self, phase: Phase, step: &mut StepContext<'_>) {
                if phase == Phase::BeforeInference {
                    step.skip_inference = true;
                }
            }
        }

        let def = AgentDefinition::new("gpt-4o-mini").with_plugin(Arc::new(SkipInferencePlugin));
        let os = AgentOs::builder().with_agent("a1", def).build().unwrap();

        let thread = Thread::with_initial_state("s", json!({}));
        let (_thread, text) = os.run_blocking("a1", thread).await.unwrap();
        assert_eq!(text, "");

        let thread = Thread::with_initial_state("s2", json!({}));
        let mut stream = os.run_stream_raw("a1", thread).unwrap();
        let ev = futures::StreamExt::next(&mut stream).await.unwrap();
        assert!(matches!(ev, AgentEvent::RunStart { .. }));
        let ev = futures::StreamExt::next(&mut stream).await.unwrap();
        assert!(matches!(ev, AgentEvent::RunFinish { .. }));
    }

    #[test]
    fn resolve_sets_runtime_caller_agent_id() {
        let os = AgentOs::builder()
            .with_agent(
                "a1",
                AgentDefinition::new("gpt-4o-mini")
                    .with_allowed_skills(vec!["s1".to_string()])
                    .with_allowed_agents(vec!["worker".to_string()])
                    .with_allowed_tools(vec!["echo".to_string()]),
            )
            .build()
            .unwrap();
        let thread = Thread::with_initial_state("s", json!({}));
        let (_client, _cfg, _tools, thread) = os.resolve("a1", thread).unwrap();
        assert_eq!(
            thread
                .runtime
                .value(RUNTIME_CALLER_AGENT_ID_KEY)
                .and_then(|v| v.as_str()),
            Some("a1")
        );
        assert_eq!(
            thread
                .runtime
                .value(crate::tool_filter::RUNTIME_ALLOWED_SKILLS_KEY),
            Some(&json!(["s1"]))
        );
        assert_eq!(
            thread
                .runtime
                .value(crate::tool_filter::RUNTIME_ALLOWED_AGENTS_KEY),
            Some(&json!(["worker"]))
        );
        assert_eq!(
            thread
                .runtime
                .value(crate::tool_filter::RUNTIME_ALLOWED_TOOLS_KEY),
            Some(&json!(["echo"]))
        );
    }

    #[tokio::test]
    async fn resolve_errors_on_skills_tool_id_conflict() {
        #[derive(Debug)]
        struct ConflictingTool;

        #[async_trait::async_trait]
        impl Tool for ConflictingTool {
            fn descriptor(&self) -> ToolDescriptor {
                ToolDescriptor::new("skill", "Conflicting", "Conflicting")
            }

            async fn execute(
                &self,
                _args: serde_json::Value,
                _ctx: &carve_state::Context<'_>,
            ) -> Result<ToolResult, ToolError> {
                Ok(ToolResult::success("skill", json!({"ok": true})))
            }
        }

        let (_td, root) = make_skills_root();
        let os = AgentOs::builder()
            .with_skills_registry(Arc::new(FsSkillRegistry::discover_root(root).unwrap()))
            .with_skills_config(SkillsConfig {
                mode: SkillsMode::DiscoveryAndRuntime,
                ..SkillsConfig::default()
            })
            .with_agent("a1", AgentDefinition::new("gpt-4o-mini"))
            .with_tools(HashMap::from([(
                "skill".to_string(),
                Arc::new(ConflictingTool) as Arc<dyn Tool>,
            )]))
            .build()
            .unwrap();

        let thread = Thread::with_initial_state("s", json!({}));
        let err = os.resolve("a1", thread).err().unwrap();
        assert!(matches!(
            err,
            AgentOsResolveError::Wiring(AgentOsWiringError::SkillsToolIdConflict(ref id))
            if id == "skill"
        ));
    }

    #[tokio::test]
    async fn resolve_wires_agent_tools_by_default() {
        let os = AgentOs::builder()
            .with_agent("a1", AgentDefinition::new("gpt-4o-mini"))
            .build()
            .unwrap();

        let thread = Thread::with_initial_state("s", json!({}));
        let (_client, cfg, tools, _thread) = os.resolve("a1", thread).unwrap();
        assert!(tools.contains_key("agent_run"));
        assert!(tools.contains_key("agent_stop"));
        assert_eq!(cfg.plugins[0].id(), "agent_tools");
        assert_eq!(cfg.plugins[1].id(), "agent_recovery");
    }

    #[tokio::test]
    async fn resolve_errors_on_agent_tools_tool_id_conflict() {
        #[derive(Debug)]
        struct ConflictingRunTool;

        #[async_trait::async_trait]
        impl Tool for ConflictingRunTool {
            fn descriptor(&self) -> ToolDescriptor {
                ToolDescriptor::new("agent_run", "Conflicting", "Conflicting")
            }

            async fn execute(
                &self,
                _args: serde_json::Value,
                _ctx: &carve_state::Context<'_>,
            ) -> Result<ToolResult, ToolError> {
                Ok(ToolResult::success("agent_run", json!({"ok": true})))
            }
        }

        let os = AgentOs::builder()
            .with_agent("a1", AgentDefinition::new("gpt-4o-mini"))
            .with_tools(HashMap::from([(
                "agent_run".to_string(),
                Arc::new(ConflictingRunTool) as Arc<dyn Tool>,
            )]))
            .build()
            .unwrap();

        let thread = Thread::with_initial_state("s", json!({}));
        let err = os.resolve("a1", thread).err().unwrap();
        assert!(matches!(
            err,
            AgentOsResolveError::Wiring(AgentOsWiringError::AgentToolIdConflict(ref id))
            if id == "agent_run"
        ));
    }

    #[test]
    fn build_errors_if_skills_enabled_without_root() {
        let err = AgentOs::builder()
            .with_skills_config(SkillsConfig {
                mode: SkillsMode::DiscoveryAndRuntime,
                ..SkillsConfig::default()
            })
            .build()
            .unwrap_err();
        assert!(matches!(err, AgentOsBuildError::SkillsNotConfigured));
    }

    #[tokio::test]
    async fn resolve_errors_if_models_registry_present_but_model_missing() {
        let os = AgentOs::builder()
            .with_provider("p1", Client::default())
            .with_model(
                "m1",
                ModelDefinition::new("p1", "gpt-4o-mini").with_chat_options(
                    genai::chat::ChatOptions::default().with_capture_usage(true),
                ),
            )
            .with_agent("a1", AgentDefinition::new("missing_model_ref"))
            .build()
            .unwrap();

        let thread = Thread::with_initial_state("s", json!({}));
        let err = os.resolve("a1", thread).err().unwrap();
        assert!(
            matches!(err, AgentOsResolveError::ModelNotFound(ref id) if id == "missing_model_ref")
        );
    }

    #[tokio::test]
    async fn resolve_rewrites_model_when_registry_present() {
        let os = AgentOs::builder()
            .with_provider("p1", Client::default())
            .with_model("m1", ModelDefinition::new("p1", "gpt-4o-mini"))
            .with_agent("a1", AgentDefinition::new("m1"))
            .build()
            .unwrap();

        let thread = Thread::with_initial_state("s", json!({}));
        let (_client, cfg, _tools, _thread) = os.resolve("a1", thread).unwrap();
        assert_eq!(cfg.model, "gpt-4o-mini");
    }

    #[derive(Debug)]
    struct TestPlugin(&'static str);

    #[async_trait]
    impl AgentPlugin for TestPlugin {
        fn id(&self) -> &str {
            self.0
        }

        async fn on_phase(&self, phase: Phase, step: &mut StepContext<'_>) {
            if phase == Phase::BeforeInference {
                step.system(format!("<plugin id=\"{}\"/>", self.0));
            }
        }
    }

    #[tokio::test]
    async fn resolve_wires_plugins_from_registry() {
        let os = AgentOs::builder()
            .with_registered_plugin("p1", Arc::new(TestPlugin("p1")))
            .with_agent(
                "a1",
                AgentDefinition::new("gpt-4o-mini").with_plugin_id("p1"),
            )
            .build()
            .unwrap();

        let thread = Thread::with_initial_state("s", json!({}));
        let (_client, cfg, _tools, _thread) = os.resolve("a1", thread.clone()).unwrap();
        assert!(cfg.plugins.iter().any(|p| p.id() == "p1"));

        let mut step = StepContext::new(&thread, vec![ToolDescriptor::new("t", "t", "t")]);
        for p in &cfg.plugins {
            p.on_phase(Phase::BeforeInference, &mut step).await;
        }
        assert!(step.system_context.iter().any(|s| s.contains("p1")));
    }

    #[tokio::test]
    async fn resolve_wires_policies_before_plugins() {
        let os = AgentOs::builder()
            .with_registered_plugin("policy1", Arc::new(TestPlugin("policy1")))
            .with_registered_plugin("p1", Arc::new(TestPlugin("p1")))
            .with_agent(
                "a1",
                AgentDefinition::new("gpt-4o-mini")
                    .with_policy_id("policy1")
                    .with_plugin_id("p1"),
            )
            .build()
            .unwrap();

        let thread = Thread::with_initial_state("s", json!({}));
        let (_client, cfg, _tools, _thread) = os.resolve("a1", thread).unwrap();
        assert_eq!(cfg.plugins[0].id(), "agent_tools");
        assert_eq!(cfg.plugins[1].id(), "agent_recovery");
        assert_eq!(cfg.plugins[2].id(), "policy1");
        assert_eq!(cfg.plugins[3].id(), "p1");
    }

    #[tokio::test]
    async fn resolve_wires_skills_before_policies_plugins_and_explicit_plugins() {
        let (_td, root) = make_skills_root();
        let os = AgentOs::builder()
            .with_skills_registry(Arc::new(FsSkillRegistry::discover_root(root).unwrap()))
            .with_skills_config(SkillsConfig {
                mode: SkillsMode::DiscoveryAndRuntime,
                ..SkillsConfig::default()
            })
            .with_registered_plugin("policy1", Arc::new(TestPlugin("policy1")))
            .with_registered_plugin("p1", Arc::new(TestPlugin("p1")))
            .with_agent(
                "a1",
                AgentDefinition::new("gpt-4o-mini")
                    .with_policy_id("policy1")
                    .with_plugin_id("p1")
                    .with_plugin(Arc::new(TestPlugin("explicit"))),
            )
            .build()
            .unwrap();

        let thread = Thread::with_initial_state("s", json!({}));
        let (_client, cfg, tools, _thread) = os.resolve("a1", thread).unwrap();
        assert!(tools.contains_key("skill"));
        assert!(tools.contains_key("load_skill_resource"));
        assert!(tools.contains_key("skill_script"));
        assert!(tools.contains_key("agent_run"));
        assert!(tools.contains_key("agent_stop"));

        assert_eq!(cfg.plugins[0].id(), "skills");
        assert_eq!(cfg.plugins[1].id(), "agent_tools");
        assert_eq!(cfg.plugins[2].id(), "agent_recovery");
        assert_eq!(cfg.plugins[3].id(), "policy1");
        assert_eq!(cfg.plugins[4].id(), "p1");
        assert_eq!(cfg.plugins[5].id(), "explicit");
    }

    #[test]
    fn build_errors_if_builder_agent_references_missing_plugin() {
        let err = AgentOs::builder()
            .with_agent(
                "a1",
                AgentDefinition::new("gpt-4o-mini").with_plugin_id("p1"),
            )
            .build()
            .unwrap_err();
        assert!(matches!(
            err,
            AgentOsBuildError::AgentPluginNotFound { ref agent_id, ref plugin_id }
            if agent_id == "a1" && plugin_id == "p1"
        ));
    }

    #[test]
    fn build_errors_if_builder_agent_references_missing_policy() {
        let err = AgentOs::builder()
            .with_agent(
                "a1",
                AgentDefinition::new("gpt-4o-mini").with_policy_id("policy1"),
            )
            .build()
            .unwrap_err();
        assert!(matches!(
            err,
            AgentOsBuildError::AgentPluginNotFound { ref agent_id, ref plugin_id }
            if agent_id == "a1" && plugin_id == "policy1"
        ));
    }

    #[test]
    fn resolve_errors_on_duplicate_plugin_id() {
        let os = AgentOs::builder()
            .with_registered_plugin("p1", Arc::new(TestPlugin("p1")))
            .with_agent(
                "a1",
                AgentDefinition::new("gpt-4o-mini")
                    .with_plugin_id("p1")
                    .with_plugin(Arc::new(TestPlugin("p1"))),
            )
            .build()
            .unwrap();

        let thread = Thread::with_initial_state("s", json!({}));
        let err = os.resolve("a1", thread).err().unwrap();
        assert!(matches!(
            err,
            AgentOsResolveError::Wiring(AgentOsWiringError::PluginAlreadyInstalled(ref id)) if id == "p1"
        ));
    }

    #[test]
    fn resolve_errors_on_duplicate_plugin_id_between_policy_and_plugin_ref() {
        let os = AgentOs::builder()
            .with_registered_plugin("p1", Arc::new(TestPlugin("p1")))
            .with_agent_registry(Arc::new({
                let mut reg = InMemoryAgentRegistry::new();
                reg.upsert(
                    "a1",
                    AgentDefinition::new("gpt-4o-mini")
                        .with_policy_id("p1")
                        .with_plugin_id("p1"),
                );
                reg
            }))
            .build()
            .unwrap();

        let thread = Thread::with_initial_state("s", json!({}));
        let err = os.resolve("a1", thread).err().unwrap();
        assert!(matches!(
            err,
            AgentOsResolveError::Wiring(AgentOsWiringError::PluginAlreadyInstalled(ref id)) if id == "p1"
        ));
    }

    #[test]
    fn build_errors_on_duplicate_plugin_ref_in_builder_agent() {
        let err = AgentOs::builder()
            .with_registered_plugin("p1", Arc::new(TestPlugin("p1")))
            .with_agent(
                "a1",
                AgentDefinition::new("gpt-4o-mini")
                    .with_policy_id("p1")
                    .with_plugin_id("p1"),
            )
            .build()
            .unwrap_err();
        assert!(matches!(
            err,
            AgentOsBuildError::AgentDuplicatePluginRef { ref agent_id, ref plugin_id }
            if agent_id == "a1" && plugin_id == "p1"
        ));
    }

    #[test]
    fn build_errors_on_reserved_plugin_id_in_builder_agent() {
        let err = AgentOs::builder()
            .with_agent(
                "a1",
                AgentDefinition::new("gpt-4o-mini").with_policy_id("skills"),
            )
            .build()
            .unwrap_err();
        assert!(matches!(
            err,
            AgentOsBuildError::AgentReservedPluginId { ref agent_id, ref plugin_id }
            if agent_id == "a1" && plugin_id == "skills"
        ));
    }

    #[test]
    fn resolve_errors_on_reserved_plugin_id() {
        let os = AgentOs::builder()
            .with_agent_registry(Arc::new({
                let mut reg = InMemoryAgentRegistry::new();
                reg.upsert(
                    "a1",
                    AgentDefinition::new("gpt-4o-mini").with_plugin_id("skills"),
                );
                reg
            }))
            .build()
            .unwrap();

        let thread = Thread::with_initial_state("s", json!({}));
        let err = os.resolve("a1", thread).err().unwrap();
        assert!(matches!(
            err,
            AgentOsResolveError::Wiring(AgentOsWiringError::ReservedPluginId(ref id)) if id == "skills"
        ));
    }

    #[test]
    fn resolve_errors_on_reserved_policy_id() {
        let os = AgentOs::builder()
            .with_agent_registry(Arc::new({
                let mut reg = InMemoryAgentRegistry::new();
                reg.upsert(
                    "a1",
                    AgentDefinition::new("gpt-4o-mini").with_policy_id("skills"),
                );
                reg
            }))
            .build()
            .unwrap();

        let thread = Thread::with_initial_state("s", json!({}));
        let err = os.resolve("a1", thread).err().unwrap();
        assert!(matches!(
            err,
            AgentOsResolveError::Wiring(AgentOsWiringError::ReservedPluginId(ref id)) if id == "skills"
        ));
    }

    #[test]
    fn build_errors_on_reserved_plugin_id_agent_tools_in_builder_agent() {
        let err = AgentOs::builder()
            .with_agent(
                "a1",
                AgentDefinition::new("gpt-4o-mini").with_plugin_id("agent_tools"),
            )
            .build()
            .unwrap_err();
        assert!(matches!(
            err,
            AgentOsBuildError::AgentReservedPluginId { ref agent_id, ref plugin_id }
            if agent_id == "a1" && plugin_id == "agent_tools"
        ));
    }

    #[test]
    fn build_errors_on_reserved_plugin_id_agent_recovery_in_builder_agent() {
        let err = AgentOs::builder()
            .with_agent(
                "a1",
                AgentDefinition::new("gpt-4o-mini").with_plugin_id("agent_recovery"),
            )
            .build()
            .unwrap_err();
        assert!(matches!(
            err,
            AgentOsBuildError::AgentReservedPluginId { ref agent_id, ref plugin_id }
            if agent_id == "a1" && plugin_id == "agent_recovery"
        ));
    }
}
