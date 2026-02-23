//! Core interaction plugins.
//!
//! This module is protocol-agnostic and operates on runtime interaction data.

mod interaction_plugin;
mod outbox;

pub const INTERACTION_PLUGIN_ID: &str = "interaction";
#[cfg(test)]
pub(crate) const RECOVERY_RESUME_TOOL_ID: &str = "agent_run";

/// Suspension action used for agent run recovery confirmation.
pub const AGENT_RECOVERY_INTERACTION_ACTION: &str = "recover_agent_run";

/// Suspension ID prefix used for agent run recovery confirmation.
pub const AGENT_RECOVERY_INTERACTION_PREFIX: &str = "agent_recovery_";

pub use interaction_plugin::InteractionPlugin;
pub use outbox::ResumeDecisionsState;
