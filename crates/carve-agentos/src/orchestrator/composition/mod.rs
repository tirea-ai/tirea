mod bundle;
mod registry;

use crate::contracts::plugin::AgentPlugin;
use crate::contracts::tool::Tool;
use crate::orchestrator::AgentDefinition;
use genai::chat::ChatOptions;
use genai::Client;
use std::collections::HashMap;
use std::sync::Arc;

pub use bundle::{
    BundleComposeError, BundleComposer, BundleRegistryAccumulator, BundleRegistryKind,
    RegistrySet, ToolPluginBundle,
};
pub use carve_agent_contract::{ToolRegistry, ToolRegistryError};
pub use registry::{
    CompositeAgentRegistry, CompositeModelRegistry, CompositePluginRegistry,
    CompositeProviderRegistry, CompositeToolRegistry, InMemoryAgentRegistry,
    InMemoryModelRegistry, InMemoryPluginRegistry, InMemoryProviderRegistry, InMemoryToolRegistry,
};

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

pub trait ModelRegistry: Send + Sync {
    fn len(&self) -> usize;

    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn get(&self, id: &str) -> Option<ModelDefinition>;

    fn ids(&self) -> Vec<String>;

    fn snapshot(&self) -> HashMap<String, ModelDefinition>;
}

/// Bundle-level contributor for registry composition.
///
/// A bundle contributes either direct definitions (maps) and/or registry sources.
/// The orchestrator composes these contributions into concrete
/// in-memory/composite registries with deterministic conflict checks.
pub trait RegistryBundle: Send + Sync {
    /// Stable bundle identifier for diagnostics.
    fn id(&self) -> &str;

    fn agent_definitions(&self) -> HashMap<String, AgentDefinition> {
        HashMap::new()
    }

    fn agent_registries(&self) -> Vec<Arc<dyn AgentRegistry>> {
        Vec::new()
    }

    fn tool_definitions(&self) -> HashMap<String, Arc<dyn Tool>> {
        HashMap::new()
    }

    fn tool_registries(&self) -> Vec<Arc<dyn ToolRegistry>> {
        Vec::new()
    }

    fn plugin_definitions(&self) -> HashMap<String, Arc<dyn AgentPlugin>> {
        HashMap::new()
    }

    fn plugin_registries(&self) -> Vec<Arc<dyn PluginRegistry>> {
        Vec::new()
    }

    fn provider_definitions(&self) -> HashMap<String, Client> {
        HashMap::new()
    }

    fn provider_registries(&self) -> Vec<Arc<dyn ProviderRegistry>> {
        Vec::new()
    }

    fn model_definitions(&self) -> HashMap<String, ModelDefinition> {
        HashMap::new()
    }

    fn model_registries(&self) -> Vec<Arc<dyn ModelRegistry>> {
        Vec::new()
    }
}
