//! Core interaction plugins.
//!
//! This module is protocol-agnostic and operates on runtime interaction data.

mod interaction_plugin;
mod interaction_response;

pub use interaction_plugin::InteractionPlugin;
