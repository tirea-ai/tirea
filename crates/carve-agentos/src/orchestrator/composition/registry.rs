use super::{
    AgentRegistry, AgentRegistryError, ModelDefinition, ModelRegistry, ModelRegistryError,
    PluginRegistry, PluginRegistryError, ProviderRegistry, ProviderRegistryError, ToolRegistry,
    ToolRegistryError,
};
use crate::contracts::agent::AgentDefinition;
use crate::contracts::extension::plugin::AgentPlugin;
use crate::contracts::extension::traits::tool::Tool;
use genai::Client;
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Clone, Default)]
pub struct InMemoryProviderRegistry {
    providers: HashMap<String, Client>,
}

impl std::fmt::Debug for InMemoryProviderRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InMemoryProviderRegistry")
            .field("len", &self.providers.len())
            .finish()
    }
}

impl InMemoryProviderRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(
        &mut self,
        provider_id: impl Into<String>,
        client: Client,
    ) -> Result<(), ProviderRegistryError> {
        let provider_id = provider_id.into();
        if provider_id.trim().is_empty() {
            return Err(ProviderRegistryError::EmptyProviderId);
        }
        if self.providers.contains_key(&provider_id) {
            return Err(ProviderRegistryError::ProviderIdConflict(provider_id));
        }
        self.providers.insert(provider_id, client);
        Ok(())
    }

    pub fn extend(
        &mut self,
        providers: HashMap<String, Client>,
    ) -> Result<(), ProviderRegistryError> {
        for (id, client) in providers {
            self.register(id, client)?;
        }
        Ok(())
    }
}

impl ProviderRegistry for InMemoryProviderRegistry {
    fn len(&self) -> usize {
        self.providers.len()
    }

    fn get(&self, id: &str) -> Option<Client> {
        self.providers.get(id).cloned()
    }

    fn ids(&self) -> Vec<String> {
        let mut ids: Vec<String> = self.providers.keys().cloned().collect();
        ids.sort();
        ids
    }

    fn snapshot(&self) -> HashMap<String, Client> {
        self.providers.clone()
    }
}

#[derive(Clone, Default)]
pub struct CompositeProviderRegistry {
    merged: InMemoryProviderRegistry,
}

impl std::fmt::Debug for CompositeProviderRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CompositeProviderRegistry")
            .field("len", &self.merged.len())
            .finish()
    }
}

impl CompositeProviderRegistry {
    pub fn try_new(
        regs: impl IntoIterator<Item = Arc<dyn ProviderRegistry>>,
    ) -> Result<Self, ProviderRegistryError> {
        let mut merged = InMemoryProviderRegistry::new();
        for r in regs {
            merged.extend(r.snapshot())?;
        }
        Ok(Self { merged })
    }
}

impl ProviderRegistry for CompositeProviderRegistry {
    fn len(&self) -> usize {
        self.merged.len()
    }

    fn get(&self, id: &str) -> Option<Client> {
        self.merged.get(id)
    }

    fn ids(&self) -> Vec<String> {
        self.merged.ids()
    }

    fn snapshot(&self) -> HashMap<String, Client> {
        self.merged.snapshot()
    }
}

#[derive(Clone, Default)]
pub struct InMemoryPluginRegistry {
    plugins: HashMap<String, Arc<dyn AgentPlugin>>,
}

impl std::fmt::Debug for InMemoryPluginRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InMemoryPluginRegistry")
            .field("len", &self.plugins.len())
            .finish()
    }
}

impl InMemoryPluginRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, plugin: Arc<dyn AgentPlugin>) -> Result<(), PluginRegistryError> {
        let id = plugin.id().to_string();
        if self.plugins.contains_key(&id) {
            return Err(PluginRegistryError::PluginIdConflict(id));
        }
        self.plugins.insert(id, plugin);
        Ok(())
    }

    pub fn register_named(
        &mut self,
        id: impl Into<String>,
        plugin: Arc<dyn AgentPlugin>,
    ) -> Result<(), PluginRegistryError> {
        let key = id.into();
        let plugin_id = plugin.id().to_string();
        if key != plugin_id {
            return Err(PluginRegistryError::PluginIdMismatch { key, plugin_id });
        }
        if self.plugins.contains_key(&key) {
            return Err(PluginRegistryError::PluginIdConflict(key));
        }
        self.plugins.insert(key, plugin);
        Ok(())
    }

    pub fn extend_named(
        &mut self,
        plugins: HashMap<String, Arc<dyn AgentPlugin>>,
    ) -> Result<(), PluginRegistryError> {
        for (key, plugin) in plugins {
            self.register_named(key, plugin)?;
        }
        Ok(())
    }

    pub fn extend_registry(
        &mut self,
        other: &dyn PluginRegistry,
    ) -> Result<(), PluginRegistryError> {
        self.extend_named(other.snapshot())
    }
}

impl PluginRegistry for InMemoryPluginRegistry {
    fn len(&self) -> usize {
        self.plugins.len()
    }

    fn get(&self, id: &str) -> Option<Arc<dyn AgentPlugin>> {
        self.plugins.get(id).cloned()
    }

    fn ids(&self) -> Vec<String> {
        let mut ids: Vec<String> = self.plugins.keys().cloned().collect();
        ids.sort();
        ids
    }

    fn snapshot(&self) -> HashMap<String, Arc<dyn AgentPlugin>> {
        self.plugins.clone()
    }
}

#[derive(Clone, Default)]
pub struct CompositePluginRegistry {
    merged: InMemoryPluginRegistry,
}

impl std::fmt::Debug for CompositePluginRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CompositePluginRegistry")
            .field("len", &self.merged.len())
            .finish()
    }
}

impl CompositePluginRegistry {
    pub fn try_new(
        regs: impl IntoIterator<Item = Arc<dyn PluginRegistry>>,
    ) -> Result<Self, PluginRegistryError> {
        let mut merged = InMemoryPluginRegistry::new();
        for r in regs {
            merged.extend_registry(r.as_ref())?;
        }
        Ok(Self { merged })
    }
}

impl PluginRegistry for CompositePluginRegistry {
    fn len(&self) -> usize {
        self.merged.len()
    }

    fn get(&self, id: &str) -> Option<Arc<dyn AgentPlugin>> {
        self.merged.get(id)
    }

    fn ids(&self) -> Vec<String> {
        self.merged.ids()
    }

    fn snapshot(&self) -> HashMap<String, Arc<dyn AgentPlugin>> {
        self.merged.snapshot()
    }
}

#[derive(Clone, Default)]
pub struct InMemoryToolRegistry {
    tools: HashMap<String, Arc<dyn Tool>>,
}

impl std::fmt::Debug for InMemoryToolRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InMemoryToolRegistry")
            .field("len", &self.tools.len())
            .finish()
    }
}

impl InMemoryToolRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.tools.len()
    }

    pub fn is_empty(&self) -> bool {
        self.tools.is_empty()
    }

    pub fn get(&self, id: &str) -> Option<Arc<dyn Tool>> {
        self.tools.get(id).cloned()
    }

    pub fn ids(&self) -> impl Iterator<Item = &String> {
        self.tools.keys()
    }

    pub fn register(&mut self, tool: Arc<dyn Tool>) -> Result<(), ToolRegistryError> {
        let id = tool.descriptor().id;
        if self.tools.contains_key(&id) {
            return Err(ToolRegistryError::ToolIdConflict(id));
        }
        self.tools.insert(id, tool);
        Ok(())
    }

    pub fn register_named(
        &mut self,
        id: impl Into<String>,
        tool: Arc<dyn Tool>,
    ) -> Result<(), ToolRegistryError> {
        let key = id.into();
        let descriptor_id = tool.descriptor().id;
        if key != descriptor_id {
            return Err(ToolRegistryError::ToolIdMismatch { key, descriptor_id });
        }
        if self.tools.contains_key(&key) {
            return Err(ToolRegistryError::ToolIdConflict(key));
        }
        self.tools.insert(key, tool);
        Ok(())
    }

    pub fn extend_named(
        &mut self,
        tools: HashMap<String, Arc<dyn Tool>>,
    ) -> Result<(), ToolRegistryError> {
        for (key, tool) in tools {
            self.register_named(key, tool)?;
        }
        Ok(())
    }

    pub fn extend_registry(&mut self, other: &dyn ToolRegistry) -> Result<(), ToolRegistryError> {
        self.extend_named(other.snapshot())
    }

    pub fn merge_many(
        regs: impl IntoIterator<Item = InMemoryToolRegistry>,
    ) -> Result<InMemoryToolRegistry, ToolRegistryError> {
        let mut out = InMemoryToolRegistry::new();
        for r in regs {
            out.extend_named(r.into_map())?;
        }
        Ok(out)
    }

    pub fn into_map(self) -> HashMap<String, Arc<dyn Tool>> {
        self.tools
    }

    pub fn to_map(&self) -> HashMap<String, Arc<dyn Tool>> {
        self.tools.clone()
    }
}

impl ToolRegistry for InMemoryToolRegistry {
    fn len(&self) -> usize {
        self.len()
    }

    fn get(&self, id: &str) -> Option<Arc<dyn Tool>> {
        self.get(id)
    }

    fn ids(&self) -> Vec<String> {
        let mut ids: Vec<String> = self.tools.keys().cloned().collect();
        ids.sort();
        ids
    }

    fn snapshot(&self) -> HashMap<String, Arc<dyn Tool>> {
        self.tools.clone()
    }
}

#[derive(Clone, Default)]
pub struct CompositeToolRegistry {
    merged: InMemoryToolRegistry,
}

impl std::fmt::Debug for CompositeToolRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CompositeToolRegistry")
            .field("len", &self.merged.len())
            .finish()
    }
}

impl CompositeToolRegistry {
    pub fn try_new(
        regs: impl IntoIterator<Item = Arc<dyn ToolRegistry>>,
    ) -> Result<Self, ToolRegistryError> {
        let mut merged = InMemoryToolRegistry::new();
        for reg in regs {
            merged.extend_registry(reg.as_ref())?;
        }
        Ok(Self { merged })
    }
}

impl ToolRegistry for CompositeToolRegistry {
    fn len(&self) -> usize {
        self.merged.len()
    }

    fn get(&self, id: &str) -> Option<Arc<dyn Tool>> {
        self.merged.get(id)
    }

    fn ids(&self) -> Vec<String> {
        let mut ids: Vec<String> = self.merged.tools.keys().cloned().collect();
        ids.sort();
        ids
    }

    fn snapshot(&self) -> HashMap<String, Arc<dyn Tool>> {
        self.merged.to_map()
    }
}

#[derive(Debug, Clone, Default)]
pub struct InMemoryAgentRegistry {
    agents: HashMap<String, AgentDefinition>,
}

impl InMemoryAgentRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.agents.len()
    }

    pub fn is_empty(&self) -> bool {
        self.agents.is_empty()
    }

    pub fn get(&self, id: &str) -> Option<AgentDefinition> {
        self.agents.get(id).cloned()
    }

    pub fn ids(&self) -> impl Iterator<Item = &String> {
        self.agents.keys()
    }

    pub fn register(
        &mut self,
        agent_id: impl Into<String>,
        mut def: AgentDefinition,
    ) -> Result<(), AgentRegistryError> {
        let agent_id = agent_id.into();
        if self.agents.contains_key(&agent_id) {
            return Err(AgentRegistryError::AgentIdConflict(agent_id));
        }
        // The registry key is canonical to avoid mismatches.
        def.id = agent_id.clone();
        self.agents.insert(agent_id, def);
        Ok(())
    }

    pub fn upsert(&mut self, agent_id: impl Into<String>, mut def: AgentDefinition) {
        let agent_id = agent_id.into();
        def.id = agent_id.clone();
        self.agents.insert(agent_id, def);
    }

    pub fn extend_upsert(&mut self, defs: HashMap<String, AgentDefinition>) {
        for (id, def) in defs {
            self.upsert(id, def);
        }
    }

    pub fn extend_registry(&mut self, other: &dyn AgentRegistry) -> Result<(), AgentRegistryError> {
        for (id, def) in other.snapshot() {
            self.register(id, def)?;
        }
        Ok(())
    }
}

impl AgentRegistry for InMemoryAgentRegistry {
    fn len(&self) -> usize {
        self.len()
    }

    fn get(&self, id: &str) -> Option<AgentDefinition> {
        self.get(id)
    }

    fn ids(&self) -> Vec<String> {
        let mut ids: Vec<String> = self.agents.keys().cloned().collect();
        ids.sort();
        ids
    }

    fn snapshot(&self) -> HashMap<String, AgentDefinition> {
        self.agents.clone()
    }
}

#[derive(Clone, Default)]
pub struct CompositeAgentRegistry {
    merged: InMemoryAgentRegistry,
}

impl std::fmt::Debug for CompositeAgentRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CompositeAgentRegistry")
            .field("len", &self.merged.len())
            .finish()
    }
}

impl CompositeAgentRegistry {
    pub fn try_new(
        regs: impl IntoIterator<Item = Arc<dyn AgentRegistry>>,
    ) -> Result<Self, AgentRegistryError> {
        let mut merged = InMemoryAgentRegistry::new();
        for r in regs {
            merged.extend_registry(r.as_ref())?;
        }
        Ok(Self { merged })
    }
}

impl AgentRegistry for CompositeAgentRegistry {
    fn len(&self) -> usize {
        self.merged.len()
    }

    fn get(&self, id: &str) -> Option<AgentDefinition> {
        self.merged.get(id)
    }

    fn ids(&self) -> Vec<String> {
        let mut ids: Vec<String> = self.merged.agents.keys().cloned().collect();
        ids.sort();
        ids
    }

    fn snapshot(&self) -> HashMap<String, AgentDefinition> {
        self.merged.agents.clone()
    }
}

#[derive(Debug, Clone, Default)]
pub struct InMemoryModelRegistry {
    models: HashMap<String, ModelDefinition>,
}

impl InMemoryModelRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.models.len()
    }

    pub fn is_empty(&self) -> bool {
        self.models.is_empty()
    }

    pub fn get(&self, id: &str) -> Option<ModelDefinition> {
        self.models.get(id).cloned()
    }

    pub fn ids(&self) -> impl Iterator<Item = &String> {
        self.models.keys()
    }

    pub fn register(
        &mut self,
        model_id: impl Into<String>,
        def: ModelDefinition,
    ) -> Result<(), ModelRegistryError> {
        let model_id = model_id.into();
        if self.models.contains_key(&model_id) {
            return Err(ModelRegistryError::ModelIdConflict(model_id));
        }
        if def.provider.trim().is_empty() {
            return Err(ModelRegistryError::EmptyProviderId);
        }
        if def.model.trim().is_empty() {
            return Err(ModelRegistryError::EmptyModelName);
        }
        self.models.insert(model_id, def);
        Ok(())
    }

    pub fn extend(
        &mut self,
        defs: HashMap<String, ModelDefinition>,
    ) -> Result<(), ModelRegistryError> {
        for (id, def) in defs {
            self.register(id, def)?;
        }
        Ok(())
    }

    pub fn extend_registry(&mut self, other: &dyn ModelRegistry) -> Result<(), ModelRegistryError> {
        self.extend(other.snapshot())
    }
}

impl ModelRegistry for InMemoryModelRegistry {
    fn len(&self) -> usize {
        self.len()
    }

    fn get(&self, id: &str) -> Option<ModelDefinition> {
        self.get(id)
    }

    fn ids(&self) -> Vec<String> {
        let mut ids: Vec<String> = self.models.keys().cloned().collect();
        ids.sort();
        ids
    }

    fn snapshot(&self) -> HashMap<String, ModelDefinition> {
        self.models.clone()
    }
}

#[derive(Clone, Default)]
pub struct CompositeModelRegistry {
    merged: InMemoryModelRegistry,
}

impl std::fmt::Debug for CompositeModelRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CompositeModelRegistry")
            .field("len", &self.merged.len())
            .finish()
    }
}

impl CompositeModelRegistry {
    pub fn try_new(
        regs: impl IntoIterator<Item = Arc<dyn ModelRegistry>>,
    ) -> Result<Self, ModelRegistryError> {
        let mut merged = InMemoryModelRegistry::new();
        for r in regs {
            merged.extend_registry(r.as_ref())?;
        }
        Ok(Self { merged })
    }
}

impl ModelRegistry for CompositeModelRegistry {
    fn len(&self) -> usize {
        self.merged.len()
    }

    fn get(&self, id: &str) -> Option<ModelDefinition> {
        self.merged.get(id)
    }

    fn ids(&self) -> Vec<String> {
        let mut ids: Vec<String> = self.merged.models.keys().cloned().collect();
        ids.sort();
        ids
    }

    fn snapshot(&self) -> HashMap<String, ModelDefinition> {
        self.merged.models.clone()
    }
}
