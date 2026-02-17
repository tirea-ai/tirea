//! Runtime protocol contracts: request, events, phase control, and outcomes.

pub mod event;
pub mod interaction;
pub mod phase;
pub mod request;
pub mod result;
pub mod state_access;
pub mod termination;

pub use event::AgentEvent;
pub use interaction::{Interaction, InteractionResponse};
pub use phase::{Phase, StepContext, StepOutcome, ToolContext};
pub use request::RunRequest;
pub use result::{StreamResult, ToolResult, ToolStatus};
pub use state_access::{ActivityContext, ActivityManager};
pub use termination::{StopReason, TerminationReason};
