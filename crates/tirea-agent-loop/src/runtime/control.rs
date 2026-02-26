//! Loop runtime state aliases.

pub use crate::contracts::io::ResumeDecisionAction;
pub use crate::contracts::runtime::run::{InferenceError, InferenceErrorState};
pub use crate::contracts::runtime::tool_call::{
    SuspendedToolCallsState, ToolCallLifecycleState, ToolCallLifecycleStatesState, ToolCallResume,
    ToolCallStatus,
};
