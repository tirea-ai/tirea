//! Execution engine internals: run context, delta, executors, policies, and control.

pub mod activity;
pub mod context;
pub mod control;
pub mod delta;
pub mod executor;
pub mod policy;
pub mod policy_scope;
pub mod result;
pub mod state_paths;

pub use activity::ActivityManager;
pub use context::RunContext;
pub use control::{
    InferenceError, InferenceErrorState, LoopControlExt, ResumeDecision, ResumeDecisionAction,
    ResumeDecisionsState, SuspendedCall, SuspendedToolCallsState, ToolCallDecision,
};
pub use delta::RunDelta;
pub use executor::{
    LlmEventStream, LlmExecutor, ToolCallOutcome, ToolExecution, ToolExecutionRequest,
    ToolExecutionResult, ToolExecutor, ToolExecutorError,
};
pub use policy::{StopPolicy, StopPolicyInput, StopPolicyStats};
pub use policy_scope::{is_id_allowed, is_scope_allowed, parse_scope_filter};
pub use result::StreamResult;
