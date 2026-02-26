//! Runtime contracts grouped by run/tool_call/llm lifecycle boundaries.

pub mod activity;
pub mod llm;
pub mod plugin;
pub mod run;
pub mod state_paths;
pub mod tool_call;

pub use activity::ActivityManager;
pub use llm::{StreamResult, TokenUsage};
#[allow(deprecated)]
pub use plugin::{
    AfterInferenceContext, AfterToolExecuteContext, AgentPlugin, BeforeInferenceContext,
    BeforeToolExecuteContext, Phase, PhasePolicy, PluginPhaseContext, ResumeInputView, RunAction,
    RunEndContext, RunLifecycleAction, RunStartContext, StateEffect, StepContext, StepEndContext,
    StepOutcome, StepStartContext, SuspendTicket, ToolCallAction, ToolCallLifecycleAction,
    ToolContext,
};
#[allow(deprecated)]
pub use run::{
    run_lifecycle_from_state, InferenceError, InferenceErrorState, RunContext, RunDelta,
    RunLifecycleState, RunLifecycleStatus, RunState, RunStatus, StoppedReason, TerminationReason,
};
#[allow(deprecated)]
pub use tool_call::{
    suspended_calls_from_state, tool_call_states_from_state, ActivityContext, DecisionReplayPolicy,
    PendingToolCall, SuspendedCall, SuspendedToolCallsState, Suspension, SuspensionResponse,
    ToolCallContext, ToolCallContextInit, ToolCallLifecycleState, ToolCallLifecycleStatesState,
    ToolCallOutcome, ToolCallResume, ToolCallResumeMode, ToolCallState, ToolCallStatesMap,
    ToolCallStatus, ToolExecution, ToolExecutionRequest, ToolExecutionResult, ToolExecutor,
    ToolExecutorError, ToolProgressState, TOOL_PROGRESS_ACTIVITY_TYPE,
};
