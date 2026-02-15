//! Core interaction plugins and intent primitives.
//!
//! This module is protocol-agnostic and operates on runtime interaction data.

mod intent;
mod interaction_plugin;
mod interaction_response;

#[cfg(test)]
pub(crate) use intent::push_pending_intent;
pub(crate) use intent::{set_pending_and_push_intent, take_intents, InteractionIntent};
pub use interaction_plugin::InteractionPlugin;
pub(crate) use interaction_response::{InteractionResolution, INTERACTION_RESOLUTIONS_KEY};
