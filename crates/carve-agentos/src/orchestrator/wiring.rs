use super::agent_tools::{
    AGENT_RECOVERY_PLUGIN_ID, AGENT_TOOLS_PLUGIN_ID, SCOPE_CALLER_AGENT_ID_KEY,
};
use super::policy::{filter_tools_in_place, set_scope_filters_from_definition_if_absent};
use super::*;
use crate::runtime::loop_runner::GenaiLlmExecutor;
use crate::extensions::skills::{
    SKILLS_BUNDLE_ID, SKILLS_DISCOVERY_PLUGIN_ID, SKILLS_PLUGIN_ID, SKILLS_RUNTIME_PLUGIN_ID,
};

#[derive(Default)]
struct ResolvedPlugins {
    global: Vec<Arc<dyn AgentPlugin>>,
    agent_default: Vec<Arc<dyn AgentPlugin>>,
    run_override: Vec<Arc<dyn AgentPlugin>>,
}

impl ResolvedPlugins {
    fn with_global(mut self, plugins: Vec<Arc<dyn AgentPlugin>>) -> Self {
        self.global.extend(plugins);
        self
    }

    fn with_agent_default(mut self, plugins: Vec<Arc<dyn AgentPlugin>>) -> Self {
        self.agent_default.extend(plugins);
        self
    }

    fn with_run_override(mut self, plugins: Vec<Arc<dyn AgentPlugin>>) -> Self {
        self.run_override.extend(plugins);
        self
    }

    fn into_plugins(self) -> Result<Vec<Arc<dyn AgentPlugin>>, AgentOsWiringError> {
        let mut plugins = Vec::new();
        plugins.extend(self.global);
        plugins.extend(self.agent_default);
        plugins.extend(self.run_override);
        AgentOs::ensure_unique_plugin_ids(&plugins)?;
        Ok(plugins)
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

    pub(super) fn reserved_plugin_ids() -> &'static [&'static str] {
        &[
            SKILLS_PLUGIN_ID,
            SKILLS_DISCOVERY_PLUGIN_ID,
            SKILLS_RUNTIME_PLUGIN_ID,
            AGENT_TOOLS_PLUGIN_ID,
            AGENT_RECOVERY_PLUGIN_ID,
        ]
    }

    fn reserved_skills_plugin_ids() -> &'static [&'static str] {
        &[
            SKILLS_PLUGIN_ID,
            SKILLS_DISCOVERY_PLUGIN_ID,
            SKILLS_RUNTIME_PLUGIN_ID,
        ]
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

    fn merge_agent_default_plugins(
        policy_plugins: Vec<Arc<dyn AgentPlugin>>,
        other_plugins: Vec<Arc<dyn AgentPlugin>>,
        explicit_plugins: Vec<Arc<dyn AgentPlugin>>,
    ) -> Vec<Arc<dyn AgentPlugin>> {
        let mut plugins = Vec::new();
        plugins.extend(policy_plugins);
        plugins.extend(other_plugins);
        plugins.extend(explicit_plugins);
        plugins
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
            if existing == AGENT_TOOLS_PLUGIN_ID {
                return Err(AgentOsWiringError::AgentToolsPluginAlreadyInstalled(
                    existing.to_string(),
                ));
            }
            if existing == AGENT_RECOVERY_PLUGIN_ID {
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

    fn build_skills_wiring_bundles(
        &self,
        explicit_plugins: &[Arc<dyn AgentPlugin>],
    ) -> Result<Vec<Arc<dyn RegistryBundle>>, AgentOsWiringError> {
        if self.skills.mode == SkillsMode::Disabled {
            return Ok(Vec::new());
        }

        Self::ensure_skills_plugin_not_installed(explicit_plugins)?;
        let reg = self
            .skills_registry
            .clone()
            .ok_or(AgentOsWiringError::SkillsNotConfigured)?;

        let mut tool_defs = HashMap::new();
        SkillSubsystem::new(reg.clone())
            .extend_tools(&mut tool_defs)
            .map_err(|e| match e {
                SkillSubsystemError::ToolIdConflict(id) => {
                    AgentOsWiringError::SkillsToolIdConflict(id)
                }
            })?;

        let mut bundle = ToolPluginBundle::new(SKILLS_BUNDLE_ID).with_tools(tool_defs);
        for plugin in self.build_skills_plugins(reg) {
            bundle = bundle.with_plugin(plugin);
        }
        Ok(vec![Arc::new(bundle)])
    }

    fn build_agent_tool_wiring_bundles(
        &self,
        explicit_plugins: &[Arc<dyn AgentPlugin>],
    ) -> Result<Vec<Arc<dyn RegistryBundle>>, AgentOsWiringError> {
        Self::ensure_agent_tools_plugin_not_installed(explicit_plugins)?;

        let run_tool: Arc<dyn Tool> =
            Arc::new(AgentRunTool::new(self.clone(), self.agent_runs.clone()));
        let stop_tool: Arc<dyn Tool> = Arc::new(AgentStopTool::new(self.agent_runs.clone()));

        let tools_plugin = AgentToolsPlugin::new(self.agents.clone(), self.agent_runs.clone())
            .with_limits(
                self.agent_tools.discovery_max_entries,
                self.agent_tools.discovery_max_chars,
            );
        let recovery_plugin = AgentRecoveryPlugin::new(self.agent_runs.clone());

        let tools_bundle: Arc<dyn RegistryBundle> = Arc::new(
            ToolPluginBundle::new(AGENT_TOOLS_PLUGIN_ID)
                .with_tool(run_tool)
                .with_tool(stop_tool)
                .with_plugin(Arc::new(tools_plugin)),
        );
        let recovery_bundle: Arc<dyn RegistryBundle> = Arc::new(
            ToolPluginBundle::new(AGENT_RECOVERY_PLUGIN_ID).with_plugin(Arc::new(recovery_plugin)),
        );

        Ok(vec![tools_bundle, recovery_bundle])
    }

    fn merge_wiring_bundles(
        &self,
        bundles: &[Arc<dyn RegistryBundle>],
        tools: &mut HashMap<String, Arc<dyn Tool>>,
        scope: WiringScope,
    ) -> Result<Vec<Arc<dyn AgentPlugin>>, AgentOsWiringError> {
        let mut plugins = Vec::new();
        for bundle in bundles {
            Self::validate_wiring_bundle(bundle.as_ref())?;
            Self::merge_wiring_bundle_tools(bundle.as_ref(), tools, scope)?;
            let mut bundle_plugins = Self::collect_wiring_bundle_plugins(bundle.as_ref())?;
            plugins.append(&mut bundle_plugins);
        }
        Self::ensure_unique_plugin_ids(&plugins)?;
        Ok(plugins)
    }

    fn validate_wiring_bundle(bundle: &dyn RegistryBundle) -> Result<(), AgentOsWiringError> {
        let unsupported = [
            (
                !bundle.agent_definitions().is_empty(),
                "agent_definitions".to_string(),
            ),
            (
                !bundle.agent_registries().is_empty(),
                "agent_registries".to_string(),
            ),
            (
                !bundle.provider_definitions().is_empty(),
                "provider_definitions".to_string(),
            ),
            (
                !bundle.provider_registries().is_empty(),
                "provider_registries".to_string(),
            ),
            (
                !bundle.model_definitions().is_empty(),
                "model_definitions".to_string(),
            ),
            (
                !bundle.model_registries().is_empty(),
                "model_registries".to_string(),
            ),
        ];
        if let Some((_, kind)) = unsupported.into_iter().find(|(has, _)| *has) {
            return Err(AgentOsWiringError::BundleUnsupportedContribution {
                bundle_id: bundle.id().to_string(),
                kind,
            });
        }
        Ok(())
    }

    fn merge_wiring_bundle_tools(
        bundle: &dyn RegistryBundle,
        tools: &mut HashMap<String, Arc<dyn Tool>>,
        scope: WiringScope,
    ) -> Result<(), AgentOsWiringError> {
        let mut defs: Vec<(String, Arc<dyn Tool>)> =
            bundle.tool_definitions().into_iter().collect();
        defs.sort_by(|a, b| a.0.cmp(&b.0));
        for (id, tool) in defs {
            if tools.contains_key(&id) {
                return Err(Self::wiring_tool_conflict(scope, bundle.id(), id));
            }
            tools.insert(id, tool);
        }

        for reg in bundle.tool_registries() {
            let mut ids = reg.ids();
            ids.sort();
            for id in ids {
                let Some(tool) = reg.get(&id) else {
                    continue;
                };
                if tools.contains_key(&id) {
                    return Err(Self::wiring_tool_conflict(scope, bundle.id(), id));
                }
                tools.insert(id, tool);
            }
        }
        Ok(())
    }

    fn collect_wiring_bundle_plugins(
        bundle: &dyn RegistryBundle,
    ) -> Result<Vec<Arc<dyn AgentPlugin>>, AgentOsWiringError> {
        let mut out = Vec::new();

        let mut defs: Vec<(String, Arc<dyn AgentPlugin>)> =
            bundle.plugin_definitions().into_iter().collect();
        defs.sort_by(|a, b| a.0.cmp(&b.0));
        for (key, plugin) in defs {
            let plugin_id = plugin.id().to_string();
            if key != plugin_id {
                return Err(AgentOsWiringError::BundlePluginIdMismatch {
                    bundle_id: bundle.id().to_string(),
                    key,
                    plugin_id,
                });
            }
            out.push(plugin);
        }

        for reg in bundle.plugin_registries() {
            let mut ids = reg.ids();
            ids.sort();
            for id in ids {
                let Some(plugin) = reg.get(&id) else {
                    continue;
                };
                if id != plugin.id() {
                    return Err(AgentOsWiringError::BundlePluginIdMismatch {
                        bundle_id: bundle.id().to_string(),
                        key: id,
                        plugin_id: plugin.id().to_string(),
                    });
                }
                out.push(plugin);
            }
        }

        Ok(out)
    }

    fn wiring_tool_conflict(scope: WiringScope, bundle_id: &str, id: String) -> AgentOsWiringError {
        match scope {
            WiringScope::RunExtension => AgentOsWiringError::RunExtensionToolIdConflict(id),
            WiringScope::System => {
                if bundle_id == SKILLS_BUNDLE_ID {
                    return AgentOsWiringError::SkillsToolIdConflict(id);
                }
                if bundle_id == AGENT_TOOLS_PLUGIN_ID || bundle_id == AGENT_RECOVERY_PLUGIN_ID {
                    return AgentOsWiringError::AgentToolIdConflict(id);
                }
                AgentOsWiringError::BundleToolIdConflict {
                    bundle_id: bundle_id.to_string(),
                    id,
                }
            }
        }
    }

    #[cfg(test)]
    pub(crate) fn wire_plugins_into(
        &self,
        mut definition: AgentDefinition,
    ) -> Result<AgentDefinition, AgentOsWiringError> {
        if definition.policy_ids.is_empty() && definition.plugin_ids.is_empty() {
            return Ok(definition);
        }

        let explicit_plugins = std::mem::take(&mut definition.plugins);
        let policy_plugins = self.resolve_plugin_id_list(&definition.policy_ids)?;
        let other_plugins = self.resolve_plugin_id_list(&definition.plugin_ids)?;
        let agent_default_plugins =
            Self::merge_agent_default_plugins(policy_plugins, other_plugins, explicit_plugins);
        definition.plugins = ResolvedPlugins::default()
            .with_agent_default(agent_default_plugins)
            .into_plugins()?;
        Ok(definition)
    }

    pub fn wire_into(
        &self,
        mut definition: AgentDefinition,
        tools: &mut HashMap<String, Arc<dyn Tool>>,
    ) -> Result<AgentConfig, AgentOsWiringError> {
        // Explicit wiring order:
        // system bundles (skills, agent_tools, agent_recovery) -> policies -> plugins -> explicit.
        let explicit_plugins = std::mem::take(&mut definition.plugins);
        let policy_plugins = self.resolve_plugin_id_list(&definition.policy_ids)?;
        let other_plugins = self.resolve_plugin_id_list(&definition.plugin_ids)?;

        let mut system_bundles = self.build_skills_wiring_bundles(&explicit_plugins)?;
        system_bundles.extend(self.build_agent_tool_wiring_bundles(&explicit_plugins)?);
        let system_plugins =
            self.merge_wiring_bundles(&system_bundles, tools, WiringScope::System)?;
        let agent_default_plugins =
            Self::merge_agent_default_plugins(policy_plugins, other_plugins, explicit_plugins);
        definition.plugins = ResolvedPlugins::default()
            .with_global(system_plugins)
            .with_agent_default(agent_default_plugins)
            .into_plugins()?;
        Ok(definition.into_loop_config())
    }

    fn resolve_model(&self, cfg: &mut AgentConfig) -> Result<(), AgentOsResolveError> {
        if self.models.is_empty() {
            cfg.llm_executor = Some(Arc::new(GenaiLlmExecutor::new(
                self.default_client.clone(),
            )));
            return Ok(());
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
        cfg.llm_executor = Some(Arc::new(GenaiLlmExecutor::new(client)));
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn wire_skills_into(
        &self,
        mut definition: AgentDefinition,
        tools: &mut HashMap<String, Arc<dyn Tool>>,
    ) -> Result<AgentDefinition, AgentOsWiringError> {
        let explicit_plugins = std::mem::take(&mut definition.plugins);
        let skills_bundles = self.build_skills_wiring_bundles(&explicit_plugins)?;
        let skills_plugins =
            self.merge_wiring_bundles(&skills_bundles, tools, WiringScope::System)?;
        definition.plugins = ResolvedPlugins::default()
            .with_global(skills_plugins)
            .with_agent_default(explicit_plugins)
            .into_plugins()?;
        Ok(definition)
    }

    /// Check whether an agent with the given ID is registered.
    pub fn validate_agent(&self, agent_id: &str) -> Result<(), AgentOsResolveError> {
        if self.agents.get(agent_id).is_some() {
            Ok(())
        } else {
            Err(AgentOsResolveError::AgentNotFound(agent_id.to_string()))
        }
    }

    pub(super) fn apply_run_extensions(
        &self,
        mut config: AgentConfig,
        mut tools: HashMap<String, Arc<dyn Tool>>,
        extensions: RunExtensions,
    ) -> Result<(AgentConfig, HashMap<String, Arc<dyn Tool>>), AgentOsWiringError> {
        let RunExtensions { bundles } = extensions;
        let extension_plugins =
            self.merge_wiring_bundles(&bundles, &mut tools, WiringScope::RunExtension)?;
        if !extension_plugins.is_empty() {
            let base_plugins = std::mem::take(&mut config.plugins);
            config.plugins = ResolvedPlugins::default()
                .with_agent_default(base_plugins)
                .with_run_override(extension_plugins)
                .into_plugins()?;
        }

        Ok((config, tools))
    }

    pub fn resolve(
        &self,
        agent_id: &str,
        mut thread: AgentState,
    ) -> Result<ResolvedAgentWiring, AgentOsResolveError> {
        let definition = self
            .agents
            .get(agent_id)
            .ok_or_else(|| AgentOsResolveError::AgentNotFound(agent_id.to_string()))?;

        if thread.scope.value(SCOPE_CALLER_AGENT_ID_KEY).is_none() {
            let _ = thread
                .scope
                .set(SCOPE_CALLER_AGENT_ID_KEY, agent_id.to_string());
        }
        let _ = set_scope_filters_from_definition_if_absent(&mut thread.scope, &definition);

        let allowed_tools = definition.allowed_tools.clone();
        let excluded_tools = definition.excluded_tools.clone();
        let mut tools = self.base_tools.snapshot();
        let mut cfg = self.wire_into(definition, &mut tools)?;
        filter_tools_in_place(
            &mut tools,
            allowed_tools.as_deref(),
            excluded_tools.as_deref(),
        );
        self.resolve_model(&mut cfg)?;
        Ok((cfg, tools, thread))
    }
}
