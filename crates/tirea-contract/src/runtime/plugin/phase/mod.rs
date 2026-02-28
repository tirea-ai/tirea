//! Phase-based plugin execution system.
//!
//! This module provides the core types for the plugin phase system:
//! - `Phase`: Execution phases in the agent loop
//! - `StepContext`: Mutable context passed through all phases
//! - `ToolGate`: Tool-call gate state in the Extensions type map

pub mod action;
pub mod core;
mod contexts;
mod extensions;
pub mod state_spec;
mod step;
mod types;

#[cfg(test)]
mod tests;

pub use action::Action;
pub use contexts::{
    AfterInferenceContext, AfterToolExecuteContext, BeforeInferenceContext,
    BeforeToolExecuteContext, PhaseContext, RunEndContext, RunStartContext, StepEndContext,
    StepStartContext,
};
pub use core::ext::ToolGate;
pub use extensions::Extensions;
pub use state_spec::{
    reduce_state_actions, AnyStateAction, CommutativeAction, StateScope, StateSpec,
};
pub use step::StepContext;
pub use types::{Phase, PhasePolicy, RunAction, StepOutcome, SuspendTicket, ToolCallAction};
