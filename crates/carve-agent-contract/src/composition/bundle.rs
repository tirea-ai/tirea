use super::{AgentRegistry, ModelDefinition, ModelRegistry, PluginRegistry, ProviderRegistry, ToolRegistry};
use crate::agent::AgentDefinition;
use crate::extension::plugin::AgentPlugin;
use crate::extension::traits::tool::Tool;
use genai::Client;
use std::collections::HashMap;
use std::sync::Arc;

/// Bundle-level contributor for registry composition.
///
/// A bundle contributes either direct definitions (maps) and/or registry sources.
/// The orchestrator is responsible for composing these contributions into concrete
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
