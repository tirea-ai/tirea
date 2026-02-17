//! Agent composition contracts: registry and bundle interfaces only.

mod bundle;
mod registry;

pub use bundle::RegistryBundle;
pub use registry::{
    AgentRegistry, AgentRegistryError, ModelDefinition, ModelRegistry, ModelRegistryError,
    PluginRegistry, PluginRegistryError, ProviderRegistry, ProviderRegistryError, ToolRegistry,
    ToolRegistryError,
};
