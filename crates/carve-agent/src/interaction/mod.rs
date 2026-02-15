//! Core interaction plugins and frontend-tool stubs.
//!
//! This module is protocol-agnostic and operates on runtime interaction data.

mod frontend_tool;
mod intent;
mod interaction_plugin;
mod interaction_response;

/// Runtime key carrying request-scoped frontend tool names.
pub use carve_agent_runtime_contract::RUNTIME_INTERACTION_FRONTEND_TOOLS_KEY;
/// Runtime key carrying request-scoped interaction responses.
pub use carve_agent_runtime_contract::RUNTIME_INTERACTION_RESPONSES_KEY;

pub(crate) use frontend_tool::{merge_frontend_tools, FrontendToolSpec};
#[cfg(test)]
pub(crate) use intent::push_pending_intent;
pub(crate) use intent::{set_pending_and_push_intent, take_intents, InteractionIntent};
pub use interaction_plugin::InteractionPlugin;
pub use interaction_plugin::InteractionPlugin as AgUiInteractionPlugin;
pub(crate) use interaction_response::{InteractionResolution, INTERACTION_RESOLUTIONS_KEY};
