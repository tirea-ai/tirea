//! Plugin contracts and phase-based execution system.

pub mod contract;
pub mod phase;

pub use contract::AgentPlugin;
pub use phase::{Phase, PhasePolicy, StepContext, StepOutcome, ToolContext};
