//! Registry traits, in-memory implementations, and agent resolution.
//!
//! See ADR-0010 for the full design rationale.

#[cfg(feature = "a2a")]
pub mod composite;
pub mod config;
pub mod memory;
pub mod resolve;
pub mod resolver;
pub mod traits;

pub use awaken_contract::registry_spec::AgentSpec;
#[cfg(feature = "a2a")]
pub use composite::{CompositeAgentSpecRegistry, DiscoveryError, RemoteAgentSource};
pub use config::{AgentSystemConfig, ModelConfig};
pub use memory::{
    MapAgentSpecRegistry, MapModelRegistry, MapPluginSource, MapProviderRegistry, MapToolRegistry,
};
pub use resolve::ResolveError;
pub use resolver::{AgentResolver, ResolvedAgent};
pub use traits::{
    AgentSpecRegistry, ModelEntry, ModelRegistry, PluginSource, ProviderRegistry, RegistrySet,
    ToolRegistry,
};
