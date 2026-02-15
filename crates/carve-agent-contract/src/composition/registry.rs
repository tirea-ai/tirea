use crate::agent::AgentDefinition;
use crate::extension::agent_plugin::AgentPlugin;
use crate::extension::traits::tool::Tool;
use genai::chat::ChatOptions;
use genai::Client;
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Debug, thiserror::Error)]
pub enum ProviderRegistryError {
    #[error("provider id already registered: {0}")]
    ProviderIdConflict(String),

    #[error("provider id must be non-empty")]
    EmptyProviderId,
}

pub trait ProviderRegistry: Send + Sync {
    fn len(&self) -> usize;

    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn get(&self, id: &str) -> Option<Client>;

    fn ids(&self) -> Vec<String>;

    fn snapshot(&self) -> HashMap<String, Client>;
}

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

#[derive(Debug, thiserror::Error)]
pub enum PluginRegistryError {
    #[error("plugin id already registered: {0}")]
    PluginIdConflict(String),

    #[error("plugin id mismatch: key={key} plugin.id()={plugin_id}")]
    PluginIdMismatch { key: String, plugin_id: String },
}

pub trait PluginRegistry: Send + Sync {
    fn len(&self) -> usize;

    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn get(&self, id: &str) -> Option<Arc<dyn AgentPlugin>>;

    fn ids(&self) -> Vec<String>;

    fn snapshot(&self) -> HashMap<String, Arc<dyn AgentPlugin>>;
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

#[derive(Debug, thiserror::Error)]
pub enum ToolRegistryError {
    #[error("tool id already registered: {0}")]
    ToolIdConflict(String),

    #[error("tool id mismatch: key={key} descriptor.id={descriptor_id}")]
    ToolIdMismatch { key: String, descriptor_id: String },
}

pub trait ToolRegistry: Send + Sync {
    fn len(&self) -> usize;

    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn get(&self, id: &str) -> Option<Arc<dyn Tool>>;

    fn ids(&self) -> Vec<String>;

    fn snapshot(&self) -> HashMap<String, Arc<dyn Tool>>;
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

#[derive(Debug, thiserror::Error)]
pub enum AgentRegistryError {
    #[error("agent id already registered: {0}")]
    AgentIdConflict(String),
}

pub trait AgentRegistry: Send + Sync {
    fn len(&self) -> usize;

    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn get(&self, id: &str) -> Option<AgentDefinition>;

    fn ids(&self) -> Vec<String>;

    fn snapshot(&self) -> HashMap<String, AgentDefinition>;
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

#[derive(Debug, Clone)]
pub struct ModelDefinition {
    pub provider: String,
    pub model: String,
    pub chat_options: Option<ChatOptions>,
}

impl ModelDefinition {
    pub fn new(provider: impl Into<String>, model: impl Into<String>) -> Self {
        Self {
            provider: provider.into(),
            model: model.into(),
            chat_options: None,
        }
    }

    pub fn with_chat_options(mut self, opts: ChatOptions) -> Self {
        self.chat_options = Some(opts);
        self
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ModelRegistryError {
    #[error("model id already registered: {0}")]
    ModelIdConflict(String),

    #[error("provider id must be non-empty")]
    EmptyProviderId,

    #[error("model name must be non-empty")]
    EmptyModelName,
}

#[derive(Debug, Clone, Default)]
pub struct InMemoryModelRegistry {
    models: HashMap<String, ModelDefinition>,
}

pub trait ModelRegistry: Send + Sync {
    fn len(&self) -> usize;

    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn get(&self, id: &str) -> Option<ModelDefinition>;

    fn ids(&self) -> Vec<String>;

    fn snapshot(&self) -> HashMap<String, ModelDefinition>;
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extension::agent_plugin::AgentPlugin;
    use crate::extension::phase::{Phase, StepContext};
    use crate::extension::traits::tool::{ToolDescriptor, ToolError, ToolResult};
    use async_trait::async_trait;
    use carve_state::Context;
    use serde_json::json;

    #[derive(Debug)]
    struct T(&'static str);

    #[async_trait]
    impl Tool for T {
        fn descriptor(&self) -> ToolDescriptor {
            ToolDescriptor::new(self.0, self.0, self.0)
        }

        async fn execute(
            &self,
            _args: serde_json::Value,
            _ctx: &carve_state::Context<'_>,
        ) -> Result<ToolResult, ToolError> {
            Ok(ToolResult::success(self.0, json!({})))
        }
    }

    #[derive(Debug)]
    struct P(&'static str);

    #[async_trait]
    impl AgentPlugin for P {
        fn id(&self) -> &str {
            self.0
        }

        async fn on_phase(&self, _phase: Phase, _step: &mut StepContext<'_>, _ctx: &Context<'_>) {}
    }

    #[test]
    fn tool_registry_register_uses_descriptor_id_and_detects_conflict() {
        let mut reg = InMemoryToolRegistry::new();
        reg.register(Arc::new(T("a"))).unwrap();
        let err = reg.register(Arc::new(T("a"))).err().unwrap();
        assert!(matches!(err, ToolRegistryError::ToolIdConflict(ref id) if id == "a"));
    }

    #[test]
    fn tool_registry_register_named_detects_mismatch() {
        let mut reg = InMemoryToolRegistry::new();
        let err = reg.register_named("x", Arc::new(T("y"))).err().unwrap();
        assert!(matches!(err, ToolRegistryError::ToolIdMismatch { .. }));
    }

    #[test]
    fn plugin_registry_register_named_detects_mismatch() {
        let mut reg = InMemoryPluginRegistry::new();
        let err = reg
            .register_named("x", Arc::new(P("y")) as Arc<dyn AgentPlugin>)
            .err()
            .unwrap();
        assert!(matches!(err, PluginRegistryError::PluginIdMismatch { .. }));
    }

    #[test]
    fn plugin_registry_register_detects_conflict() {
        let mut reg = InMemoryPluginRegistry::new();
        reg.register(Arc::new(P("p1"))).unwrap();
        let err = reg.register(Arc::new(P("p1"))).err().unwrap();
        assert!(matches!(
            err,
            PluginRegistryError::PluginIdConflict(ref id) if id == "p1"
        ));
    }

    #[test]
    fn agent_registry_register_canonicalizes_id() {
        let mut reg = InMemoryAgentRegistry::new();
        reg.register("a1", AgentDefinition::with_id("not-a1", "gpt-4o-mini"))
            .unwrap();
        let def = reg.get("a1").unwrap();
        assert_eq!(def.id, "a1");
    }

    #[test]
    fn model_registry_register_rejects_empty_model_name() {
        let mut reg = InMemoryModelRegistry::new();
        let err = reg
            .register("m1", ModelDefinition::new("p1", "   "))
            .err()
            .unwrap();
        assert!(matches!(err, ModelRegistryError::EmptyModelName));
    }

    #[test]
    fn model_registry_register_rejects_empty_provider_id() {
        let mut reg = InMemoryModelRegistry::new();
        let err = reg
            .register("m1", ModelDefinition::new("   ", "gpt-4o-mini"))
            .err()
            .unwrap();
        assert!(matches!(err, ModelRegistryError::EmptyProviderId));
    }

    #[test]
    fn tool_registry_merge_many_detects_conflict() {
        let mut a = InMemoryToolRegistry::new();
        a.register(Arc::new(T("x"))).unwrap();
        let mut b = InMemoryToolRegistry::new();
        b.register(Arc::new(T("x"))).unwrap();
        let err = InMemoryToolRegistry::merge_many([a, b]).err().unwrap();
        assert!(matches!(err, ToolRegistryError::ToolIdConflict(ref id) if id == "x"));
    }

    #[test]
    fn composite_tool_registry_detects_conflict() {
        let mut a = InMemoryToolRegistry::new();
        a.register(Arc::new(T("x"))).unwrap();
        let mut b = InMemoryToolRegistry::new();
        b.register(Arc::new(T("x"))).unwrap();

        let err = CompositeToolRegistry::try_new([
            Arc::new(a) as Arc<dyn ToolRegistry>,
            Arc::new(b) as Arc<dyn ToolRegistry>,
        ])
        .err()
        .unwrap();

        assert!(matches!(err, ToolRegistryError::ToolIdConflict(ref id) if id == "x"));
    }
}
