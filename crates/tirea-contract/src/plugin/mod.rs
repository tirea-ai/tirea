//! Plugin contracts and phase-based execution system.

pub mod contract;
pub mod phase;

pub use contract::AgentPlugin;
pub use phase::{
    AfterInferenceContext, AfterToolExecuteContext, BeforeInferenceContext,
    BeforeToolExecuteContext, Phase, PhasePolicy, PluginPhaseContext, ResumeInputView, RunAction,
    RunEndContext, RunStartContext, StateEffect, StepContext, StepEndContext, StepOutcome,
    StepStartContext, SuspendTicket, ToolAction, ToolContext, ToolGateDecision,
};
