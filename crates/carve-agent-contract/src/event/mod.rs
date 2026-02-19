//! Run lifecycle events, interactions, and termination reasons.

pub mod interaction;
pub mod stream;
pub mod termination;

pub use interaction::{Interaction, InteractionResponse};
pub use stream::{
    clear_runtime_event_envelope_meta, register_runtime_event_envelope_meta, AgentEvent,
};
pub use termination::{StopConditionSpec, StopReason, TerminationReason};
