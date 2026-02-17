use super::*;

fn merge_registry<R: ?Sized, M>(
    memory: M,
    mut external: Vec<Arc<R>>,
    is_memory_empty: impl Fn(&M) -> bool,
    into_memory_registry: impl FnOnce(M) -> Arc<R>,
    compose: impl FnOnce(Vec<Arc<R>>) -> Result<Arc<R>, AgentOsBuildError>,
) -> Result<Arc<R>, AgentOsBuildError> {
    if external.is_empty() {
        return Ok(into_memory_registry(memory));
    }

    let mut registries: Vec<Arc<R>> = Vec::new();
    if !is_memory_empty(&memory) {
        registries.push(into_memory_registry(memory));
    }
    registries.append(&mut external);

    if registries.len() == 1 {
        Ok(registries.pop().expect("single registry must exist"))
    } else {
        compose(registries)
    }
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
            .field("agent_state_store", &self.agent_state_store.is_some())
            .finish()
    }
}

impl std::fmt::Debug for AgentOsBuilder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AgentOsBuilder")
            .field("client", &self.client.is_some())
            .field("bundles", &self.bundles.len())
            .field("agents", &self.agents.len())
            .field("base_tools", &self.base_tools.len())
            .field("plugins", &self.plugins.len())
            .field("providers", &self.providers.len())
            .field("models", &self.models.len())
            .field("skills_registry", &self.skills_registry.is_some())
            .field("skills", &self.skills)
            .field("agent_tools", &self.agent_tools)
            .field("agent_state_store", &self.agent_state_store.is_some())
            .finish()
    }
}

impl AgentOsBuilder {
    pub fn new() -> Self {
        Self {
            client: None,
            bundles: Vec::new(),
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
            agent_state_store: None,
        }
    }

    pub fn with_client(mut self, client: Client) -> Self {
        self.client = Some(client);
        self
    }

    pub fn with_bundle(mut self, bundle: Arc<dyn RegistryBundle>) -> Self {
        self.bundles.push(bundle);
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

    pub fn with_agent_state_store(mut self, agent_state_store: Arc<dyn AgentStateStore>) -> Self {
        self.agent_state_store = Some(agent_state_store);
        self
    }

    pub fn build(self) -> Result<AgentOs, AgentOsBuildError> {
        let AgentOsBuilder {
            client,
            bundles,
            agents: mut agents_defs,
            mut agent_registries,
            base_tools: mut base_tools_defs,
            mut base_tool_registries,
            plugins: mut plugin_defs,
            mut plugin_registries,
            providers: mut provider_defs,
            mut provider_registries,
            models: mut model_defs,
            mut model_registries,
            skills_registry,
            skills,
            agent_tools,
            agent_state_store,
        } = self;

        BundleComposer::apply(
            &bundles,
            BundleRegistryAccumulator {
                agent_definitions: &mut agents_defs,
                agent_registries: &mut agent_registries,
                tool_definitions: &mut base_tools_defs,
                tool_registries: &mut base_tool_registries,
                plugin_definitions: &mut plugin_defs,
                plugin_registries: &mut plugin_registries,
                provider_definitions: &mut provider_defs,
                provider_registries: &mut provider_registries,
                model_definitions: &mut model_defs,
                model_registries: &mut model_registries,
            },
        )?;

        if skills.mode != SkillsMode::Disabled && skills_registry.is_none() {
            return Err(AgentOsBuildError::SkillsNotConfigured);
        }

        let mut base_tools = InMemoryToolRegistry::new();
        base_tools.extend_named(base_tools_defs)?;

        let base_tools: Arc<dyn ToolRegistry> = merge_registry(
            base_tools,
            base_tool_registries,
            |reg: &InMemoryToolRegistry| reg.is_empty(),
            |reg| Arc::new(reg),
            |regs| Ok(Arc::new(CompositeToolRegistry::try_new(regs)?)),
        )?;

        let mut plugins = InMemoryPluginRegistry::new();
        plugins.extend_named(plugin_defs)?;

        let plugins: Arc<dyn PluginRegistry> = merge_registry(
            plugins,
            plugin_registries,
            |reg: &InMemoryPluginRegistry| reg.is_empty(),
            |reg| Arc::new(reg),
            |regs| Ok(Arc::new(CompositePluginRegistry::try_new(regs)?)),
        )?;

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

        let providers: Arc<dyn ProviderRegistry> = merge_registry(
            providers,
            provider_registries,
            |reg: &InMemoryProviderRegistry| reg.is_empty(),
            |reg| Arc::new(reg),
            |regs| Ok(Arc::new(CompositeProviderRegistry::try_new(regs)?)),
        )?;

        let mut models = InMemoryModelRegistry::new();
        models.extend(model_defs.clone())?;

        let models: Arc<dyn ModelRegistry> = merge_registry(
            models,
            model_registries,
            |reg: &InMemoryModelRegistry| reg.is_empty(),
            |reg| Arc::new(reg),
            |regs| Ok(Arc::new(CompositeModelRegistry::try_new(regs)?)),
        )?;

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

        let agents: Arc<dyn AgentRegistry> = merge_registry(
            agents,
            agent_registries,
            |reg: &InMemoryAgentRegistry| reg.is_empty(),
            |reg| Arc::new(reg),
            |regs| Ok(Arc::new(CompositeAgentRegistry::try_new(regs)?)),
        )?;

        let registries = RegistrySet::new(
            agents,
            base_tools,
            plugins,
            providers,
            models,
        );

        Ok(AgentOs {
            default_client: client.unwrap_or_default(),
            agents: registries.agents,
            base_tools: registries.tools,
            plugins: registries.plugins,
            providers: registries.providers,
            models: registries.models,
            skills_registry,
            skills,
            agent_runs: Arc::new(AgentRunManager::new()),
            agent_tools,
            agent_state_store,
        })
    }
}

impl Default for AgentOsBuilder {
    fn default() -> Self {
        Self::new()
    }
}
