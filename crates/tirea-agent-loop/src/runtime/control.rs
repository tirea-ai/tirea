//! Loop control-state — re-exported from [`tirea_contract::runtime::control`].

pub use crate::contracts::runtime::control::{
    InferenceError, InferenceErrorState, ResumeDecision, ResumeDecisionAction,
    ResumeDecisionsState, SuspendedCallsExt, SuspendedToolCallsState, ToolCallResume,
    ToolCallState, ToolCallStatesState, ToolCallStatus,
};
