//! Phase-based plugin execution system.
//!
//! This module provides the core types for the plugin phase system:
//! - `Phase`: Execution phases in the agent loop
//! - `StepContext`: Mutable context passed through all phases
//! - `ToolContext`: Tool-call state carried by `StepContext`

mod contexts;
mod step;
mod types;

#[cfg(test)]
mod tests;

pub use contexts::{
    AfterInferenceContext, AfterToolExecuteContext, BeforeInferenceContext,
    BeforeToolExecuteContext, PluginPhaseContext, ResumeInputView, RunEndContext, RunStartContext,
    StepEndContext, StepStartContext,
};
pub use step::{StepContext, ToolContext};
pub use types::{
    Phase, PhasePolicy, RunLifecycleAction, StateEffect, StepOutcome, SuspendTicket,
    ToolCallLifecycleAction,
};
