//! Core interaction plugins and frontend-tool stubs.
//!
//! This module is protocol-agnostic and operates on runtime interaction data.

mod frontend_tool;
mod interaction_plugin;
mod interaction_response;

pub(crate) use frontend_tool::{
    merge_frontend_tools, FrontendToolSpec,
};
pub use interaction_plugin::AgUiInteractionPlugin;

#[cfg(test)]
pub(crate) use frontend_tool::{FrontendToolPlugin, FrontendToolStub};
#[cfg(test)]
pub(crate) use interaction_response::InteractionResponsePlugin;
