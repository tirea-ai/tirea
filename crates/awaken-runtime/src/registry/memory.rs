//! In-memory HashMap-backed registry implementations.

use std::collections::HashMap;
use std::sync::Arc;

use crate::builder::BuildError;
use crate::plugins::Plugin;
use awaken_contract::contract::executor::LlmExecutor;
use awaken_contract::contract::tool::Tool;

use super::traits::{
    AgentSpecRegistry, ModelEntry, ModelRegistry, PluginSource, ProviderRegistry, ToolRegistry,
};
use awaken_contract::registry_spec::AgentSpec;

// ---------------------------------------------------------------------------
// MapToolRegistry
// ---------------------------------------------------------------------------

/// In-memory tool registry backed by a `HashMap`.
#[derive(Default)]
pub struct MapToolRegistry {
    tools: HashMap<String, Arc<dyn Tool>>,
}

impl MapToolRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(
        &mut self,
        id: impl Into<String>,
        tool: Arc<dyn Tool>,
    ) -> Result<(), BuildError> {
        let id = id.into();
        if self.tools.contains_key(&id) {
            return Err(BuildError::ToolRegistryConflict(format!(
                "tool '{}' already registered",
                id
            )));
        }
        self.tools.insert(id, tool);
        Ok(())
    }
}

impl ToolRegistry for MapToolRegistry {
    fn get_tool(&self, id: &str) -> Option<Arc<dyn Tool>> {
        self.tools.get(id).cloned()
    }

    fn tool_ids(&self) -> Vec<String> {
        self.tools.keys().cloned().collect()
    }
}

// ---------------------------------------------------------------------------
// MapModelRegistry
// ---------------------------------------------------------------------------

/// In-memory model registry backed by a `HashMap`.
#[derive(Default)]
pub struct MapModelRegistry {
    models: HashMap<String, ModelEntry>,
}

impl MapModelRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, id: impl Into<String>, entry: ModelEntry) -> Result<(), BuildError> {
        let id = id.into();
        if self.models.contains_key(&id) {
            return Err(BuildError::ModelRegistryConflict(format!(
                "model '{}' already registered",
                id
            )));
        }
        self.models.insert(id, entry);
        Ok(())
    }
}

impl ModelRegistry for MapModelRegistry {
    fn get_model(&self, id: &str) -> Option<&ModelEntry> {
        self.models.get(id)
    }
}

// ---------------------------------------------------------------------------
// MapProviderRegistry
// ---------------------------------------------------------------------------

/// In-memory provider registry backed by a `HashMap`.
#[derive(Default)]
pub struct MapProviderRegistry {
    providers: HashMap<String, Arc<dyn LlmExecutor>>,
}

impl MapProviderRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(
        &mut self,
        id: impl Into<String>,
        executor: Arc<dyn LlmExecutor>,
    ) -> Result<(), BuildError> {
        let id = id.into();
        if self.providers.contains_key(&id) {
            return Err(BuildError::ProviderRegistryConflict(format!(
                "provider '{}' already registered",
                id
            )));
        }
        self.providers.insert(id, executor);
        Ok(())
    }
}

impl ProviderRegistry for MapProviderRegistry {
    fn get_provider(&self, id: &str) -> Option<Arc<dyn LlmExecutor>> {
        self.providers.get(id).cloned()
    }
}

// ---------------------------------------------------------------------------
// MapAgentSpecRegistry
// ---------------------------------------------------------------------------

/// In-memory agent spec registry backed by a `HashMap`.
#[derive(Default)]
pub struct MapAgentSpecRegistry {
    agents: HashMap<String, AgentSpec>,
}

impl MapAgentSpecRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, spec: AgentSpec) -> Result<(), BuildError> {
        if self.agents.contains_key(&spec.id) {
            return Err(BuildError::AgentRegistryConflict(format!(
                "agent '{}' already registered",
                spec.id
            )));
        }
        self.agents.insert(spec.id.clone(), spec);
        Ok(())
    }
}

impl AgentSpecRegistry for MapAgentSpecRegistry {
    fn get_agent(&self, id: &str) -> Option<AgentSpec> {
        self.agents.get(id).cloned()
    }

    fn agent_ids(&self) -> Vec<String> {
        self.agents.keys().cloned().collect()
    }
}

// ---------------------------------------------------------------------------
// MapPluginSource
// ---------------------------------------------------------------------------

/// In-memory plugin source backed by a `HashMap`.
#[derive(Default)]
pub struct MapPluginSource {
    plugins: HashMap<String, Arc<dyn Plugin>>,
}

impl MapPluginSource {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(
        &mut self,
        id: impl Into<String>,
        plugin: Arc<dyn Plugin>,
    ) -> Result<(), BuildError> {
        let id = id.into();
        if self.plugins.contains_key(&id) {
            return Err(BuildError::PluginRegistryConflict(format!(
                "plugin '{}' already registered",
                id
            )));
        }
        self.plugins.insert(id, plugin);
        Ok(())
    }
}

impl PluginSource for MapPluginSource {
    fn get_plugin(&self, id: &str) -> Option<Arc<dyn Plugin>> {
        self.plugins.get(id).cloned()
    }
}
