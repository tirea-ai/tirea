//! Runtime input/output contracts and command DTOs.

pub mod decision;
pub mod event;
pub mod input;

pub use decision::{ResumeDecisionAction, ToolCallDecision};
pub use event::AgentEvent;
pub use event::AgentEvent as RuntimeOutput;
pub use input::{RunRequest, RuntimeInput};
