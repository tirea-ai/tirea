use crate::agent::AgentDefinition;
use crate::extension::plugin::AgentPlugin;
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
