//! Core interaction plugins.
//!
//! This module is protocol-agnostic and operates on runtime interaction data.

mod context_provider;
mod interaction_plugin;
mod interaction_response;
mod outbox;

pub const INTERACTION_PLUGIN_ID: &str = "interaction";
pub const INTERACTION_RESPONSE_PLUGIN_ID: &str = "interaction_response";
pub(crate) const RECOVERY_RESUME_TOOL_ID: &str = "agent_run";

/// Interaction action used for agent run recovery confirmation.
pub const AGENT_RECOVERY_INTERACTION_ACTION: &str = "recover_agent_run";

/// Interaction ID prefix used for agent run recovery confirmation.
pub const AGENT_RECOVERY_INTERACTION_PREFIX: &str = "agent_recovery_";

pub use context_provider::{ContextCategory, ContextProvider};
pub use interaction_plugin::InteractionPlugin;
pub use outbox::InteractionOutbox;
