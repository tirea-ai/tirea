use crate::event::termination::TerminationReason;
use crate::tool::contract::ToolResult;
use crate::runtime::result::TokenUsage;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tirea_state::TrackedPatch;

/// Agent loop events for streaming execution.
#[derive(Debug, Clone)]
pub enum AgentEvent {
    /// Run started.
    RunStart {
        thread_id: String,
        run_id: String,
        parent_run_id: Option<String>,
    },
    /// Run finished.
    RunFinish {
        thread_id: String,
        run_id: String,
        result: Option<Value>,
        /// Why this run terminated.
        termination: TerminationReason,
    },

    /// LLM text delta.
    TextDelta { delta: String },
    /// LLM reasoning delta.
    ReasoningDelta { delta: String },
    /// Opaque reasoning signature/token delta from provider.
    ReasoningEncryptedValue { encrypted_value: String },

    /// Tool call started.
    ToolCallStart { id: String, name: String },
    /// Tool call arguments delta.
    ToolCallDelta { id: String, args_delta: String },
    /// Tool call input is complete.
    ToolCallReady {
        id: String,
        name: String,
        arguments: Value,
    },
    /// Tool call completed.
    ToolCallDone {
        id: String,
        result: ToolResult,
        patch: Option<TrackedPatch>,
        /// Pre-generated ID for the stored tool result message.
        message_id: String,
    },

    /// Step started.
    StepStart {
        /// Pre-generated ID for the assistant message this step will produce.
        message_id: String,
    },
    /// Step completed.
    StepEnd,

    /// LLM inference completed with token usage data.
    InferenceComplete {
        /// Model used for this inference.
        model: String,
        /// Token usage.
        usage: Option<TokenUsage>,
        /// Duration of the LLM call in milliseconds.
        duration_ms: u64,
    },

    /// State snapshot.
    StateSnapshot { snapshot: Value },
    /// State delta.
    StateDelta { delta: Vec<Value> },
    /// Messages snapshot.
    MessagesSnapshot { messages: Vec<Value> },

    /// Activity snapshot.
    ActivitySnapshot {
        message_id: String,
        activity_type: String,
        content: Value,
        replace: Option<bool>,
    },
    /// Activity delta.
    ActivityDelta {
        message_id: String,
        activity_type: String,
        patch: Vec<Value>,
    },

    /// Suspension resolution received.
    ToolCallResumed { target_id: String, result: Value },

    /// Error occurred.
    Error { message: String },
}

impl AgentEvent {
    /// Extract the response text from a `RunFinish` result value.
    pub fn extract_response(result: &Option<Value>) -> String {
        result
            .as_ref()
            .and_then(|v| v.get("response"))
            .and_then(|r| r.as_str())
            .unwrap_or_default()
            .to_string()
    }

    pub(crate) fn event_type(&self) -> AgentEventType {
        match self {
            Self::RunStart { .. } => AgentEventType::RunStart,
            Self::RunFinish { .. } => AgentEventType::RunFinish,
            Self::TextDelta { .. } => AgentEventType::TextDelta,
            Self::ReasoningDelta { .. } => AgentEventType::ReasoningDelta,
            Self::ReasoningEncryptedValue { .. } => AgentEventType::ReasoningEncryptedValue,
            Self::ToolCallStart { .. } => AgentEventType::ToolCallStart,
            Self::ToolCallDelta { .. } => AgentEventType::ToolCallDelta,
            Self::ToolCallReady { .. } => AgentEventType::ToolCallReady,
            Self::ToolCallDone { .. } => AgentEventType::ToolCallDone,
            Self::StepStart { .. } => AgentEventType::StepStart,
            Self::StepEnd => AgentEventType::StepEnd,
            Self::InferenceComplete { .. } => AgentEventType::InferenceComplete,
            Self::StateSnapshot { .. } => AgentEventType::StateSnapshot,
            Self::StateDelta { .. } => AgentEventType::StateDelta,
            Self::MessagesSnapshot { .. } => AgentEventType::MessagesSnapshot,
            Self::ActivitySnapshot { .. } => AgentEventType::ActivitySnapshot,
            Self::ActivityDelta { .. } => AgentEventType::ActivityDelta,
            Self::ToolCallResumed { .. } => AgentEventType::ToolCallResumed,
            Self::Error { .. } => AgentEventType::Error,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum AgentEventType {
    RunStart,
    RunFinish,
    TextDelta,
    ReasoningDelta,
    ReasoningEncryptedValue,
    ToolCallStart,
    ToolCallDelta,
    ToolCallReady,
    ToolCallDone,
    StepStart,
    StepEnd,
    InferenceComplete,
    StateSnapshot,
    StateDelta,
    MessagesSnapshot,
    ActivitySnapshot,
    ActivityDelta,
    ToolCallResumed,
    Error,
}
