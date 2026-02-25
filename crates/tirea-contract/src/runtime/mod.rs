//! Runtime contracts and durable state schemas.
//!
//! - **Contracts**: `RunContext`, `ToolExecutor`, `ActivityManager`, `RunDelta`, `StreamResult`
//! - **Durable state**: `RunLifecycleState`, `SuspendedToolCallsState`, `ToolCallStatesState`,
//!   `InferenceErrorState` — persisted under reserved `__*` thread-state paths
//! - **Utilities**: scope filter helpers (`is_scope_allowed`, `parse_scope_filter`)

pub mod activity;
pub mod context;
pub mod control;
pub mod delta;
pub mod executor;
pub mod policy_scope;
pub mod result;
pub mod state_paths;

pub use activity::ActivityManager;
pub use context::RunContext;
pub use control::{
    InferenceError, InferenceErrorState, ResumeDecisionAction, RunLifecycleState,
    RunLifecycleStatus, SuspendedCall, SuspendedToolCallsState, ToolCallDecision, ToolCallResume,
    ToolCallState, ToolCallStatesState, ToolCallStatus,
};
pub use delta::RunDelta;
pub use executor::{
    DecisionReplayPolicy, ToolCallOutcome, ToolExecution, ToolExecutionRequest,
    ToolExecutionResult, ToolExecutor, ToolExecutorError,
};
pub use policy_scope::{is_id_allowed, is_scope_allowed, parse_scope_filter};
pub use result::{StreamResult, TokenUsage};
