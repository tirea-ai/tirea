use crate::runtime::interaction::Interaction;
use crate::runtime::result::ToolResult;
use crate::runtime::termination::TerminationReason;
use carve_state::TrackedPatch;
use genai::chat::Usage;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Agent loop events for streaming execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "event_type", rename_all = "snake_case")]
pub enum AgentEvent {
    /// Run started.
    RunStart {
        thread_id: String,
        run_id: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        parent_run_id: Option<String>,
    },
    /// Run finished.
    RunFinish {
        thread_id: String,
        run_id: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        result: Option<Value>,
        /// Why this run terminated.
        termination: TerminationReason,
    },

    /// LLM text delta.
    TextDelta { delta: String },

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
        #[serde(default)]
        message_id: String,
    },

    /// Step started.
    StepStart {
        /// Pre-generated ID for the assistant message this step will produce.
        #[serde(default)]
        message_id: String,
    },
    /// Step completed.
    StepEnd,

    /// LLM inference completed with token usage data.
    InferenceComplete {
        /// Model used for this inference.
        model: String,
        /// Token usage.
        #[serde(skip_serializing_if = "Option::is_none")]
        usage: Option<Usage>,
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

    /// Interaction request created.
    InteractionRequested { interaction: Interaction },
    /// Interaction resolution received.
    InteractionResolved {
        interaction_id: String,
        result: Value,
    },
    /// Pending interaction request.
    Pending { interaction: Interaction },

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
}
