//! Runtime protocol contracts: request, events, phase control, and outcomes.

pub mod event;
pub mod interaction;
pub mod policy_scope;
pub mod phase;
pub mod request;
pub mod result;
pub mod termination;

pub use crate::state::{ActivityContext, ActivityManager};
pub use event::AgentEvent;
pub use interaction::{Interaction, InteractionResponse};
pub use policy_scope::{
    SCOPE_ALLOWED_AGENTS_KEY, SCOPE_ALLOWED_SKILLS_KEY, SCOPE_ALLOWED_TOOLS_KEY,
    SCOPE_EXCLUDED_AGENTS_KEY, SCOPE_EXCLUDED_SKILLS_KEY, SCOPE_EXCLUDED_TOOLS_KEY,
};
pub use phase::{Phase, StepContext, StepOutcome, ToolContext};
pub use request::RunRequest;
pub use result::{StreamResult, ToolResult, ToolStatus};
pub use termination::{StopConditionSpec, StopReason, TerminationReason};
