pub mod contract;
pub mod phase;

pub use contract::AgentPlugin;
#[allow(deprecated)]
pub use phase::{
    AfterInferenceContext, AfterToolExecuteContext, BeforeInferenceContext,
    BeforeToolExecuteContext, Phase, PhasePolicy, PluginPhaseContext, ResumeInputView, RunAction,
    RunEndContext, RunLifecycleAction, RunStartContext, StateEffect, StepContext, StepEndContext,
    StepOutcome, StepStartContext, SuspendTicket, ToolCallAction, ToolCallLifecycleAction,
    ToolContext,
};
