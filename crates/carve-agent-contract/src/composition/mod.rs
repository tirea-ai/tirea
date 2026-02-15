//! Agent composition contracts: registries and bundles.

mod bundle;
mod registry;

pub use bundle::{
    BundleComposeError, BundleComposer, BundleRegistryAccumulator, BundleRegistryKind,
    RegistryBundle, RegistrySet, ToolPluginBundle,
};
pub use registry::{
    AgentRegistry, AgentRegistryError, CompositeAgentRegistry, CompositeModelRegistry,
    CompositePluginRegistry, CompositeProviderRegistry, CompositeToolRegistry,
    InMemoryAgentRegistry, InMemoryModelRegistry, InMemoryPluginRegistry, InMemoryProviderRegistry,
    InMemoryToolRegistry, ModelDefinition, ModelRegistry, ModelRegistryError, PluginRegistry,
    PluginRegistryError, ProviderRegistry, ProviderRegistryError, ToolRegistry, ToolRegistryError,
};
