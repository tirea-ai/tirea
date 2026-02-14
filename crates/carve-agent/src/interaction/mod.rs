//! Core interaction plugins and frontend-tool stubs.
//!
//! This module is protocol-agnostic and operates on runtime interaction data.

mod frontend_tool;
mod intent;
mod interaction_plugin;
mod interaction_response;

/// Runtime key carrying request-scoped frontend tool names.
pub const RUNTIME_INTERACTION_FRONTEND_TOOLS_KEY: &str = "interaction_frontend_tools";
/// Runtime key carrying request-scoped interaction responses.
pub const RUNTIME_INTERACTION_RESPONSES_KEY: &str = "interaction_responses";

pub(crate) use frontend_tool::{FrontendToolRegistry, FrontendToolSpec};
pub(crate) use intent::{push_pending_intent, take_intents, InteractionIntent};
pub use interaction_plugin::InteractionPlugin;
pub use interaction_plugin::InteractionPlugin as AgUiInteractionPlugin;

#[cfg(test)]
pub(crate) use frontend_tool::{FrontendToolPlugin, FrontendToolStub};
#[cfg(test)]
pub(crate) use interaction_response::InteractionResponsePlugin;
pub(crate) use interaction_response::{InteractionResolution, INTERACTION_RESOLUTIONS_KEY};
