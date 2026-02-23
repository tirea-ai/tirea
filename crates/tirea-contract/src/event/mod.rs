//! Run lifecycle events, suspensions, and termination reasons.

pub mod stream;
pub mod suspension;
pub mod termination;

pub use stream::{
    clear_runtime_event_envelope_meta, register_runtime_event_envelope_meta, AgentEvent,
};
pub use suspension::{
    FrontendToolInvocation, InvocationOrigin, ResponseRouting, Suspension, SuspensionResponse,
};
pub use termination::{StopConditionSpec, StopReason, TerminationReason};
