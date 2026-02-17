//! Core interaction plugins.
//!
//! This module is protocol-agnostic and operates on runtime interaction data.

mod interaction_plugin;
mod interaction_response;
mod agent_control;
mod context_provider;

pub const INTERACTION_PLUGIN_ID: &str = "interaction";
pub const INTERACTION_RESPONSE_PLUGIN_ID: &str = "interaction_response";
pub(crate) const RECOVERY_RESUME_TOOL_ID: &str = "agent_run";

pub use interaction_plugin::InteractionPlugin;
pub use context_provider::{ContextCategory, ContextProvider};
