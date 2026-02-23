//! Run lifecycle events, suspensions, and termination reasons.

pub mod suspension;
pub mod stream;
pub mod termination;

pub use suspension::{
    FrontendToolInvocation, Suspension, SuspensionResponse, InvocationOrigin, ResponseRouting,
};
pub use stream::{
    clear_runtime_event_envelope_meta, register_runtime_event_envelope_meta, AgentEvent,
};
pub use termination::{StopConditionSpec, StopReason, TerminationReason};
