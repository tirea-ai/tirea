//! Runtime input/output contracts and command DTOs.

pub mod decision;
pub mod decision_translation;
pub mod event;
pub mod input;

pub use decision::{ResumeDecisionAction, ToolCallDecision};
pub use decision_translation::suspension_response_to_decision;
pub use event::AgentEvent;
pub use event::AgentEvent as RuntimeOutput;
pub use input::{RunRequest, RuntimeInput};
