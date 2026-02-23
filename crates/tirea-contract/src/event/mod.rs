//! Run lifecycle events, suspensions, and termination reasons.

pub mod stream;
pub mod suspension;
pub mod termination;

pub use stream::AgentEvent;
pub use suspension::{
    FrontendToolInvocation, InvocationOrigin, ResponseRouting, Suspension, SuspensionResponse,
};
pub use termination::{StoppedReason, TerminationReason};
