//! Runtime contracts and durable state schemas.
//!
//! - **Contracts**: `RunContext`, `ToolExecutor`, `ActivityManager`, `RunDelta`, `StreamResult`
//! - **Durable state**: `RunLifecycleState`, `SuspendedToolCallsState`, `ToolCallStatesState`,
//!   `InferenceErrorState` — persisted under reserved `__*` thread-state paths

pub mod activity;
pub mod context;
pub mod control;
pub mod delta;
pub mod executor;
pub mod result;
pub mod state_paths;

pub use activity::ActivityManager;
pub use context::RunContext;
pub use control::{
    InferenceError, InferenceErrorState, PendingToolCall, ResumeDecisionAction, RunLifecycleState,
    RunLifecycleStatus, SuspendedCall, SuspendedToolCallsState, ToolCallDecision, ToolCallResume,
    ToolCallResumeMode, ToolCallState, ToolCallStatesState, ToolCallStatus,
};
pub use delta::RunDelta;
pub use executor::{
    DecisionReplayPolicy, ToolCallOutcome, ToolExecution, ToolExecutionRequest,
    ToolExecutionResult, ToolExecutor, ToolExecutorError,
};
pub use result::{StreamResult, TokenUsage};
