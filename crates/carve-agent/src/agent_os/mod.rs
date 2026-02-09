use crate::plugin::AgentPlugin;
use crate::r#loop::{run_loop, run_loop_stream};
mod registry;
use crate::skills::{
    SkillDiscoveryPlugin, SkillPlugin, SkillRegistry, SkillRuntimePlugin, SkillSubsystem,
    SkillSubsystemError,
};
use crate::traits::tool::Tool;
use crate::{AgentConfig, AgentDefinition, AgentEvent, AgentLoopError, Session};
use genai::Client;
pub use registry::{
    AgentRegistry, AgentRegistryError, CompositeAgentRegistry, CompositeModelRegistry,
    CompositeProviderRegistry, CompositeToolRegistry, InMemoryAgentRegistry, InMemoryModelRegistry,
    InMemoryProviderRegistry, InMemoryToolRegistry, ModelDefinition, ModelRegistry, ModelRegistryError,
    ProviderRegistry, ProviderRegistryError, ToolRegistry, ToolRegistryError,
    PluginRegistry, PluginRegistryError, InMemoryPluginRegistry, CompositePluginRegistry,
};
use std::collections::HashMap;
use std::sync::Arc;

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

    #[error("models configured but no ProviderRegistry configured")]
    ProvidersNotConfigured,

    #[error("provider not found: {provider_id} (for model id: {model_id})")]
    ProviderNotFound { provider_id: String, model_id: String },

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
    ProviderNotFound { provider_id: String, model_id: String },

    #[error(transparent)]
    Wiring(#[from] AgentOsWiringError),
}

#[derive(Debug, thiserror::Error)]
pub enum AgentOsRunError {
    #[error(transparent)]
    Resolve(#[from] AgentOsResolveError),

    #[error(transparent)]
    Loop(#[from] AgentLoopError),
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

    pub fn agent(&self, agent_id: &str) -> Option<AgentDefinition> {
        self.agents.get(agent_id)
    }

    pub fn tools(&self) -> HashMap<String, Arc<dyn Tool>> {
        self.base_tools.snapshot()
    }

    fn reserved_plugin_ids() -> &'static [&'static str] {
        &["skills", "skills_discovery", "skills_runtime"]
    }

    pub fn wire_plugins_into(
        &self,
        mut config: AgentConfig,
    ) -> Result<AgentConfig, AgentOsWiringError> {
        if config.policy_ids.is_empty() && config.plugin_ids.is_empty() {
            return Ok(config);
        }

        let reserved = Self::reserved_plugin_ids();
        let mut out: Vec<Arc<dyn AgentPlugin>> = Vec::new();

        for id in &config.policy_ids {
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

        for id in &config.plugin_ids {
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

        // Append explicitly-provided plugins.
        out.extend(config.plugins);

        // Fail-fast: plugins are keyed by `AgentPlugin::id()`. Duplicates are almost always a bug.
        let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
        for p in &out {
            let id = p.id().to_string();
            if !seen.insert(id.clone()) {
                return Err(AgentOsWiringError::PluginAlreadyInstalled(id));
            }
        }

        config.plugins = out;
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
        if self.skills.mode == SkillsMode::Disabled {
            return Ok(config);
        }

        let reg = self
            .skills_registry
            .clone()
            .ok_or(AgentOsWiringError::SkillsNotConfigured)?;

        // Prevent duplicate plugin installation.
        let reserved_plugin_ids = ["skills", "skills_discovery", "skills_runtime"];
        if let Some(existing) = config
            .plugins
            .iter()
            .map(|p| p.id())
            .find(|id| reserved_plugin_ids.contains(id))
        {
            return Err(AgentOsWiringError::SkillsPluginAlreadyInstalled(
                existing.to_string(),
            ));
        }

        // Register skills tools.
        let skills = SkillSubsystem::new(reg.clone());
        skills.extend_tools(tools).map_err(|e| match e {
            SkillSubsystemError::ToolIdConflict(id) => AgentOsWiringError::SkillsToolIdConflict(id),
        })?;

        // Register skills plugins.
        let mut plugins: Vec<Arc<dyn AgentPlugin>> = Vec::new();
        match self.skills.mode {
            SkillsMode::Disabled => {}
            SkillsMode::DiscoveryAndRuntime => {
                let discovery = SkillDiscoveryPlugin::new(reg).with_limits(
                    self.skills.discovery_max_entries,
                    self.skills.discovery_max_chars,
                );
                plugins.push(SkillPlugin::new(discovery).boxed());
            }
            SkillsMode::DiscoveryOnly => {
                let discovery = SkillDiscoveryPlugin::new(reg).with_limits(
                    self.skills.discovery_max_entries,
                    self.skills.discovery_max_chars,
                );
                plugins.push(Arc::new(discovery));
            }
            SkillsMode::RuntimeOnly => {
                plugins.push(Arc::new(SkillRuntimePlugin::new()));
            }
        }

        // Prepend skills plugins so skill context appears early and ordering is deterministic.
        plugins.extend(config.plugins);
        config.plugins = plugins;

        Ok(config)
    }

    pub fn resolve(
        &self,
        agent_id: &str,
        session: Session,
    ) -> Result<(Client, AgentConfig, HashMap<String, Arc<dyn Tool>>, Session), AgentOsResolveError>
    {
        let def = self
            .agents
            .get(agent_id)
            .ok_or_else(|| AgentOsResolveError::AgentNotFound(agent_id.to_string()))?;

        let mut tools = self.base_tools.snapshot();
        let cfg = self.wire_plugins_into(def)?;
        let mut cfg = self.wire_skills_into(cfg, &mut tools)?;
        let client = self.resolve_model(&mut cfg)?;
        Ok((client, cfg, tools, session))
    }

    pub async fn run(
        &self,
        agent_id: &str,
        session: Session,
    ) -> Result<(Session, String), AgentOsRunError> {
        let (client, cfg, tools, session) = self.resolve(agent_id, session)?;
        let (session, text) = run_loop(&client, &cfg, session, &tools).await?;
        Ok((session, text))
    }

    pub fn run_stream(
        &self,
        agent_id: &str,
        session: Session,
    ) -> Result<impl futures::Stream<Item = AgentEvent> + Send, AgentOsResolveError> {
        let (client, cfg, tools, session) = self.resolve(agent_id, session)?;
        Ok(run_loop_stream(client, cfg, session, tools))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::phase::{Phase, StepContext};
    use crate::session::Session;
    use crate::skills::FsSkillRegistry;
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
        assert!(tools.contains_key("load_skill_reference"));
        assert!(tools.contains_key("skill_script"));

        assert_eq!(cfg.plugins.len(), 1);
        assert_eq!(cfg.plugins[0].id(), "skills");

        // Verify injection does not panic and includes catalog.
        let session = Session::with_initial_state(
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
        let mut step = StepContext::new(&session, vec![ToolDescriptor::new("t", "t", "t")]);
        cfg.plugins[0]
            .on_phase(Phase::BeforeInference, &mut step)
            .await;
        assert_eq!(step.system_context.len(), 2);
        assert!(step.system_context[0].contains("<available_skills>"));
        assert!(step.system_context[1].contains("<skill id=\"s1\">"));
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

        let session = Session::with_initial_state(
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
        let mut step = StepContext::new(&session, vec![ToolDescriptor::new("t", "t", "t")]);
        cfg.plugins[0]
            .on_phase(Phase::BeforeInference, &mut step)
            .await;
        assert_eq!(step.system_context.len(), 1);
        assert!(step.system_context[0].contains("<skill id=\"s1\">"));
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

    #[test]
    fn resolve_errors_if_agent_missing() {
        let os = AgentOs::builder().build().unwrap();
        let session = Session::with_initial_state("s", json!({}));
        let err = os.resolve("missing", session).err().unwrap();
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

        let session = Session::with_initial_state("s", json!({}));
        let (_client, cfg, tools, _session) = os.resolve("a1", session).unwrap();

        assert_eq!(cfg.id, "a1");
        assert!(tools.contains_key("base_tool"));
        assert!(tools.contains_key("skill"));
        assert!(tools.contains_key("load_skill_reference"));
        assert!(tools.contains_key("skill_script"));
        assert_eq!(cfg.plugins.len(), 1);
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

        let session = Session::with_initial_state("s", json!({}));
        let (_session, text) = os.run("a1", session).await.unwrap();
        assert_eq!(text, "");

        let session = Session::with_initial_state("s2", json!({}));
        let mut stream = os.run_stream("a1", session).unwrap();
        let ev = futures::StreamExt::next(&mut stream).await.unwrap();
        assert!(matches!(ev, AgentEvent::Done { .. }));
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

        let session = Session::with_initial_state("s", json!({}));
        let err = os.resolve("a1", session).err().unwrap();
        assert!(matches!(
            err,
            AgentOsResolveError::Wiring(AgentOsWiringError::SkillsToolIdConflict(ref id))
            if id == "skill"
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

        let session = Session::with_initial_state("s", json!({}));
        let err = os.resolve("a1", session).err().unwrap();
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

        let session = Session::with_initial_state("s", json!({}));
        let (_client, cfg, _tools, _session) = os.resolve("a1", session).unwrap();
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
            .with_agent("a1", AgentDefinition::new("gpt-4o-mini").with_plugin_id("p1"))
            .build()
            .unwrap();

        let session = Session::with_initial_state("s", json!({}));
        let (_client, cfg, _tools, _session) = os.resolve("a1", session.clone()).unwrap();
        assert!(cfg.plugins.iter().any(|p| p.id() == "p1"));

        let mut step = StepContext::new(&session, vec![ToolDescriptor::new("t", "t", "t")]);
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

        let session = Session::with_initial_state("s", json!({}));
        let (_client, cfg, _tools, _session) = os.resolve("a1", session).unwrap();
        assert_eq!(cfg.plugins[0].id(), "policy1");
        assert_eq!(cfg.plugins[1].id(), "p1");
    }

    #[test]
    fn resolve_errors_if_plugin_missing() {
        let os = AgentOs::builder()
            .with_agent("a1", AgentDefinition::new("gpt-4o-mini").with_plugin_id("p1"))
            .build()
            .unwrap();

        let session = Session::with_initial_state("s", json!({}));
        let err = os.resolve("a1", session).err().unwrap();
        assert!(matches!(
            err,
            AgentOsResolveError::Wiring(AgentOsWiringError::PluginNotFound(ref id)) if id == "p1"
        ));
    }

    #[test]
    fn resolve_errors_if_policy_missing() {
        let os = AgentOs::builder()
            .with_agent("a1", AgentDefinition::new("gpt-4o-mini").with_policy_id("policy1"))
            .build()
            .unwrap();

        let session = Session::with_initial_state("s", json!({}));
        let err = os.resolve("a1", session).err().unwrap();
        assert!(matches!(
            err,
            AgentOsResolveError::Wiring(AgentOsWiringError::PluginNotFound(ref id)) if id == "policy1"
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

        let session = Session::with_initial_state("s", json!({}));
        let err = os.resolve("a1", session).err().unwrap();
        assert!(matches!(
            err,
            AgentOsResolveError::Wiring(AgentOsWiringError::PluginAlreadyInstalled(ref id)) if id == "p1"
        ));
    }

    #[test]
    fn resolve_errors_on_duplicate_plugin_id_between_policy_and_plugin_ref() {
        let os = AgentOs::builder()
            .with_registered_plugin("p1", Arc::new(TestPlugin("p1")))
            .with_agent(
                "a1",
                AgentDefinition::new("gpt-4o-mini")
                    .with_policy_id("p1")
                    .with_plugin_id("p1"),
            )
            .build()
            .unwrap();

        let session = Session::with_initial_state("s", json!({}));
        let err = os.resolve("a1", session).err().unwrap();
        assert!(matches!(
            err,
            AgentOsResolveError::Wiring(AgentOsWiringError::PluginAlreadyInstalled(ref id)) if id == "p1"
        ));
    }

    #[test]
    fn resolve_errors_on_reserved_plugin_id() {
        let os = AgentOs::builder()
            .with_agent(
                "a1",
                AgentDefinition::new("gpt-4o-mini").with_plugin_id("skills"),
            )
            .build()
            .unwrap();

        let session = Session::with_initial_state("s", json!({}));
        let err = os.resolve("a1", session).err().unwrap();
        assert!(matches!(
            err,
            AgentOsResolveError::Wiring(AgentOsWiringError::ReservedPluginId(ref id)) if id == "skills"
        ));
    }

    #[test]
    fn resolve_errors_on_reserved_policy_id() {
        let os = AgentOs::builder()
            .with_agent(
                "a1",
                AgentDefinition::new("gpt-4o-mini").with_policy_id("skills"),
            )
            .build()
            .unwrap();

        let session = Session::with_initial_state("s", json!({}));
        let err = os.resolve("a1", session).err().unwrap();
        assert!(matches!(
            err,
            AgentOsResolveError::Wiring(AgentOsWiringError::ReservedPluginId(ref id)) if id == "skills"
        ));
    }
}
