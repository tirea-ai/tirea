//! Resolution errors.

/// Errors from the resolution process.
#[derive(Debug, thiserror::Error)]
pub enum ResolveError {
    #[error("agent not found: {0}")]
    AgentNotFound(String),
    #[error("model not found: {0}")]
    ModelNotFound(String),
    #[error("provider not found: {0}")]
    ProviderNotFound(String),
    #[error("plugin not found: {0}")]
    PluginNotFound(String),
    #[error("invalid config for plugin {plugin}: section \"{key}\" — {message}")]
    InvalidPluginConfig {
        plugin: String,
        key: String,
        message: String,
    },
    #[error("remote agent `{0}` cannot be resolved locally — use it as a delegate instead")]
    RemoteAgentNotDirectlyRunnable(String),
    #[error("env build error: {0}")]
    EnvBuild(#[from] awaken_contract::StateError),
}
