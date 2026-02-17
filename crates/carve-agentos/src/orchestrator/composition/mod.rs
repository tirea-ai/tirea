mod bundle;
mod registry;

pub use bundle::{
    BundleComposeError, BundleComposer, BundleRegistryAccumulator, BundleRegistryKind,
    RegistrySet, ToolPluginBundle,
};
pub use carve_agent_contract::composition::{
    AgentRegistry, AgentRegistryError, ModelDefinition, ModelRegistry, ModelRegistryError,
    PluginRegistry, PluginRegistryError, ProviderRegistry, ProviderRegistryError, RegistryBundle,
    ToolRegistry, ToolRegistryError,
};
pub use registry::{
    CompositeAgentRegistry, CompositeModelRegistry, CompositePluginRegistry,
    CompositeProviderRegistry, CompositeToolRegistry, InMemoryAgentRegistry,
    InMemoryModelRegistry, InMemoryPluginRegistry, InMemoryProviderRegistry, InMemoryToolRegistry,
};
