//! Core interaction plugins.
//!
//! This module is protocol-agnostic and operates on runtime interaction data.

mod agent_control;
mod context_provider;
mod interaction_plugin;
mod interaction_response;

pub const INTERACTION_PLUGIN_ID: &str = "interaction";
pub const INTERACTION_RESPONSE_PLUGIN_ID: &str = "interaction_response";
pub(crate) const RECOVERY_RESUME_TOOL_ID: &str = "agent_run";

pub use context_provider::{ContextCategory, ContextProvider};
pub use interaction_plugin::InteractionPlugin;
