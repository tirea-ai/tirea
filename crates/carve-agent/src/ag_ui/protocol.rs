use crate::state_types::Interaction;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;

// Base Event Fields
// ============================================================================

/// Common fields for all AG-UI events (BaseEvent).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct BaseEventFields {
    /// Event timestamp in milliseconds since epoch.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<u64>,
    /// Raw event data from external systems.
    #[serde(rename = "rawEvent", skip_serializing_if = "Option::is_none")]
    pub raw_event: Option<Value>,
}

// ============================================================================
// Message Role
// ============================================================================

/// Role for text messages.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum MessageRole {
    Developer,
    System,
    #[default]
    Assistant,
    User,
    Tool,
}

// ============================================================================
// AG-UI Event Types
// ============================================================================

/// AG-UI Protocol Event Types.
///
/// These events follow the AG-UI specification for agent-to-frontend communication.
/// See: <https://docs.ag-ui.com/concepts/events>
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type")]
pub enum AGUIEvent {
    // ========================================================================
    // Lifecycle Events
    // ========================================================================
    /// Signals the start of an agent run.
    #[serde(rename = "RUN_STARTED")]
    RunStarted {
        #[serde(rename = "threadId")]
        thread_id: String,
        #[serde(rename = "runId")]
        run_id: String,
        #[serde(rename = "parentRunId", skip_serializing_if = "Option::is_none")]
        parent_run_id: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        input: Option<Value>,
        #[serde(flatten)]
        base: BaseEventFields,
    },

    /// Signals successful completion of an agent run.
    #[serde(rename = "RUN_FINISHED")]
    RunFinished {
        #[serde(rename = "threadId")]
        thread_id: String,
        #[serde(rename = "runId")]
        run_id: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        result: Option<Value>,
        #[serde(flatten)]
        base: BaseEventFields,
    },

    /// Indicates an error occurred during the run.
    #[serde(rename = "RUN_ERROR")]
    RunError {
        message: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        code: Option<String>,
        #[serde(flatten)]
        base: BaseEventFields,
    },

    /// Marks the beginning of a step within a run.
    #[serde(rename = "STEP_STARTED")]
    StepStarted {
        #[serde(rename = "stepName")]
        step_name: String,
        #[serde(flatten)]
        base: BaseEventFields,
    },

    /// Marks the completion of a step.
    #[serde(rename = "STEP_FINISHED")]
    StepFinished {
        #[serde(rename = "stepName")]
        step_name: String,
        #[serde(flatten)]
        base: BaseEventFields,
    },

    // ========================================================================
    // Text Message Events
    // ========================================================================
    /// Indicates the beginning of a text message stream.
    #[serde(rename = "TEXT_MESSAGE_START")]
    TextMessageStart {
        #[serde(rename = "messageId")]
        message_id: String,
        /// Role is always "assistant" for TEXT_MESSAGE_START.
        role: MessageRole,
        #[serde(flatten)]
        base: BaseEventFields,
    },

    /// Contains incremental text content.
    #[serde(rename = "TEXT_MESSAGE_CONTENT")]
    TextMessageContent {
        #[serde(rename = "messageId")]
        message_id: String,
        delta: String,
        #[serde(flatten)]
        base: BaseEventFields,
    },

    /// Indicates the end of a text message stream.
    #[serde(rename = "TEXT_MESSAGE_END")]
    TextMessageEnd {
        #[serde(rename = "messageId")]
        message_id: String,
        #[serde(flatten)]
        base: BaseEventFields,
    },

    /// Combined chunk event for text messages (alternative to Start/Content/End).
    #[serde(rename = "TEXT_MESSAGE_CHUNK")]
    TextMessageChunk {
        #[serde(rename = "messageId", skip_serializing_if = "Option::is_none")]
        message_id: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        role: Option<MessageRole>,
        #[serde(skip_serializing_if = "Option::is_none")]
        delta: Option<String>,
        #[serde(flatten)]
        base: BaseEventFields,
    },

    // ========================================================================
    // Tool Call Events
    // ========================================================================
    /// Signals the start of a tool call.
    #[serde(rename = "TOOL_CALL_START")]
    ToolCallStart {
        #[serde(rename = "toolCallId")]
        tool_call_id: String,
        #[serde(rename = "toolCallName")]
        tool_call_name: String,
        #[serde(rename = "parentMessageId", skip_serializing_if = "Option::is_none")]
        parent_message_id: Option<String>,
        #[serde(flatten)]
        base: BaseEventFields,
    },

    /// Contains incremental tool arguments.
    #[serde(rename = "TOOL_CALL_ARGS")]
    ToolCallArgs {
        #[serde(rename = "toolCallId")]
        tool_call_id: String,
        delta: String,
        #[serde(flatten)]
        base: BaseEventFields,
    },

    /// Signals the end of tool argument streaming.
    #[serde(rename = "TOOL_CALL_END")]
    ToolCallEnd {
        #[serde(rename = "toolCallId")]
        tool_call_id: String,
        #[serde(flatten)]
        base: BaseEventFields,
    },

    /// Contains the result of a tool execution.
    #[serde(rename = "TOOL_CALL_RESULT")]
    ToolCallResult {
        #[serde(rename = "messageId")]
        message_id: String,
        #[serde(rename = "toolCallId")]
        tool_call_id: String,
        content: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        role: Option<MessageRole>,
        #[serde(flatten)]
        base: BaseEventFields,
    },

    /// Combined chunk event for tool calls (alternative to Start/Args/End).
    #[serde(rename = "TOOL_CALL_CHUNK")]
    ToolCallChunk {
        #[serde(rename = "toolCallId", skip_serializing_if = "Option::is_none")]
        tool_call_id: Option<String>,
        #[serde(rename = "toolCallName", skip_serializing_if = "Option::is_none")]
        tool_call_name: Option<String>,
        #[serde(rename = "parentMessageId", skip_serializing_if = "Option::is_none")]
        parent_message_id: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        delta: Option<String>,
        #[serde(flatten)]
        base: BaseEventFields,
    },

    // ========================================================================
    // State Management Events
    // ========================================================================
    /// Provides a complete state snapshot.
    #[serde(rename = "STATE_SNAPSHOT")]
    StateSnapshot {
        snapshot: Value,
        #[serde(flatten)]
        base: BaseEventFields,
    },

    /// Contains incremental state changes (RFC 6902 JSON Patch).
    #[serde(rename = "STATE_DELTA")]
    StateDelta {
        /// Array of JSON Patch operations.
        delta: Vec<Value>,
        #[serde(flatten)]
        base: BaseEventFields,
    },

    /// Provides a complete message history snapshot.
    #[serde(rename = "MESSAGES_SNAPSHOT")]
    MessagesSnapshot {
        messages: Vec<Value>,
        #[serde(flatten)]
        base: BaseEventFields,
    },

    // ========================================================================
    // Activity Events
    // ========================================================================
    /// Provides an activity snapshot.
    #[serde(rename = "ACTIVITY_SNAPSHOT")]
    ActivitySnapshot {
        #[serde(rename = "messageId")]
        message_id: String,
        #[serde(rename = "activityType")]
        activity_type: String,
        content: HashMap<String, Value>,
        #[serde(skip_serializing_if = "Option::is_none")]
        replace: Option<bool>,
        #[serde(flatten)]
        base: BaseEventFields,
    },

    /// Contains incremental activity changes (RFC 6902 JSON Patch).
    #[serde(rename = "ACTIVITY_DELTA")]
    ActivityDelta {
        #[serde(rename = "messageId")]
        message_id: String,
        #[serde(rename = "activityType")]
        activity_type: String,
        /// Array of JSON Patch operations.
        patch: Vec<Value>,
        #[serde(flatten)]
        base: BaseEventFields,
    },

    // ========================================================================
    // Special Events
    // ========================================================================
    /// Wraps events from external systems.
    #[serde(rename = "RAW")]
    Raw {
        event: Value,
        #[serde(skip_serializing_if = "Option::is_none")]
        source: Option<String>,
        #[serde(flatten)]
        base: BaseEventFields,
    },

    /// Custom application-defined event.
    #[serde(rename = "CUSTOM")]
    Custom {
        name: String,
        value: Value,
        #[serde(flatten)]
        base: BaseEventFields,
    },
}

impl AGUIEvent {
    // ========================================================================
    // Factory Methods - Lifecycle
    // ========================================================================

    /// Create a run-started event.
    pub fn run_started(
        thread_id: impl Into<String>,
        run_id: impl Into<String>,
        parent_run_id: Option<String>,
    ) -> Self {
        Self::RunStarted {
            thread_id: thread_id.into(),
            run_id: run_id.into(),
            parent_run_id,
            input: None,
            base: BaseEventFields::default(),
        }
    }

    /// Create a run-started event with input.
    pub fn run_started_with_input(
        thread_id: impl Into<String>,
        run_id: impl Into<String>,
        parent_run_id: Option<String>,
        input: Value,
    ) -> Self {
        Self::RunStarted {
            thread_id: thread_id.into(),
            run_id: run_id.into(),
            parent_run_id,
            input: Some(input),
            base: BaseEventFields::default(),
        }
    }

    /// Create a run-finished event.
    pub fn run_finished(
        thread_id: impl Into<String>,
        run_id: impl Into<String>,
        result: Option<Value>,
    ) -> Self {
        Self::RunFinished {
            thread_id: thread_id.into(),
            run_id: run_id.into(),
            result,
            base: BaseEventFields::default(),
        }
    }

    /// Create a run-error event.
    pub fn run_error(message: impl Into<String>, code: Option<String>) -> Self {
        Self::RunError {
            message: message.into(),
            code,
            base: BaseEventFields::default(),
        }
    }

    /// Create a step-started event.
    pub fn step_started(step_name: impl Into<String>) -> Self {
        Self::StepStarted {
            step_name: step_name.into(),
            base: BaseEventFields::default(),
        }
    }

    /// Create a step-finished event.
    pub fn step_finished(step_name: impl Into<String>) -> Self {
        Self::StepFinished {
            step_name: step_name.into(),
            base: BaseEventFields::default(),
        }
    }

    // ========================================================================
    // Factory Methods - Text Message
    // ========================================================================

    /// Create a text-message-start event.
    pub fn text_message_start(message_id: impl Into<String>) -> Self {
        Self::TextMessageStart {
            message_id: message_id.into(),
            role: MessageRole::Assistant,
            base: BaseEventFields::default(),
        }
    }

    /// Create a text-message-content event.
    pub fn text_message_content(message_id: impl Into<String>, delta: impl Into<String>) -> Self {
        Self::TextMessageContent {
            message_id: message_id.into(),
            delta: delta.into(),
            base: BaseEventFields::default(),
        }
    }

    /// Create a text-message-end event.
    pub fn text_message_end(message_id: impl Into<String>) -> Self {
        Self::TextMessageEnd {
            message_id: message_id.into(),
            base: BaseEventFields::default(),
        }
    }

    /// Create a text-message-chunk event.
    pub fn text_message_chunk(
        message_id: Option<String>,
        role: Option<MessageRole>,
        delta: Option<String>,
    ) -> Self {
        Self::TextMessageChunk {
            message_id,
            role,
            delta,
            base: BaseEventFields::default(),
        }
    }

    // ========================================================================
    // Factory Methods - Tool Call
    // ========================================================================

    /// Create a tool-call-start event.
    pub fn tool_call_start(
        tool_call_id: impl Into<String>,
        tool_call_name: impl Into<String>,
        parent_message_id: Option<String>,
    ) -> Self {
        Self::ToolCallStart {
            tool_call_id: tool_call_id.into(),
            tool_call_name: tool_call_name.into(),
            parent_message_id,
            base: BaseEventFields::default(),
        }
    }

    /// Create a tool-call-args event.
    pub fn tool_call_args(tool_call_id: impl Into<String>, delta: impl Into<String>) -> Self {
        Self::ToolCallArgs {
            tool_call_id: tool_call_id.into(),
            delta: delta.into(),
            base: BaseEventFields::default(),
        }
    }

    /// Create a tool-call-end event.
    pub fn tool_call_end(tool_call_id: impl Into<String>) -> Self {
        Self::ToolCallEnd {
            tool_call_id: tool_call_id.into(),
            base: BaseEventFields::default(),
        }
    }

    /// Create a tool-call-result event.
    pub fn tool_call_result(
        message_id: impl Into<String>,
        tool_call_id: impl Into<String>,
        content: impl Into<String>,
    ) -> Self {
        Self::ToolCallResult {
            message_id: message_id.into(),
            tool_call_id: tool_call_id.into(),
            content: content.into(),
            role: Some(MessageRole::Tool),
            base: BaseEventFields::default(),
        }
    }

    /// Create a tool-call-chunk event.
    pub fn tool_call_chunk(
        tool_call_id: Option<String>,
        tool_call_name: Option<String>,
        parent_message_id: Option<String>,
        delta: Option<String>,
    ) -> Self {
        Self::ToolCallChunk {
            tool_call_id,
            tool_call_name,
            parent_message_id,
            delta,
            base: BaseEventFields::default(),
        }
    }

    // ========================================================================
    // Factory Methods - State
    // ========================================================================

    /// Create a state-snapshot event.
    pub fn state_snapshot(snapshot: Value) -> Self {
        Self::StateSnapshot {
            snapshot,
            base: BaseEventFields::default(),
        }
    }

    /// Create a state-delta event.
    pub fn state_delta(delta: Vec<Value>) -> Self {
        Self::StateDelta {
            delta,
            base: BaseEventFields::default(),
        }
    }

    /// Create a messages-snapshot event.
    pub fn messages_snapshot(messages: Vec<Value>) -> Self {
        Self::MessagesSnapshot {
            messages,
            base: BaseEventFields::default(),
        }
    }

    // ========================================================================
    // Factory Methods - Activity
    // ========================================================================

    /// Create an activity-snapshot event.
    pub fn activity_snapshot(
        message_id: impl Into<String>,
        activity_type: impl Into<String>,
        content: HashMap<String, Value>,
        replace: Option<bool>,
    ) -> Self {
        Self::ActivitySnapshot {
            message_id: message_id.into(),
            activity_type: activity_type.into(),
            content,
            replace,
            base: BaseEventFields::default(),
        }
    }

    /// Create an activity-delta event.
    pub fn activity_delta(
        message_id: impl Into<String>,
        activity_type: impl Into<String>,
        patch: Vec<Value>,
    ) -> Self {
        Self::ActivityDelta {
            message_id: message_id.into(),
            activity_type: activity_type.into(),
            patch,
            base: BaseEventFields::default(),
        }
    }

    // ========================================================================
    // Factory Methods - Special
    // ========================================================================

    /// Create a raw event.
    pub fn raw(event: Value, source: Option<String>) -> Self {
        Self::Raw {
            event,
            source,
            base: BaseEventFields::default(),
        }
    }

    /// Create a custom event.
    pub fn custom(name: impl Into<String>, value: Value) -> Self {
        Self::Custom {
            name: name.into(),
            value,
            base: BaseEventFields::default(),
        }
    }

    // ========================================================================
    // Utility Methods
    // ========================================================================

    /// Set timestamp on the event.
    pub fn with_timestamp(mut self, timestamp: u64) -> Self {
        match &mut self {
            Self::RunStarted { base, .. }
            | Self::RunFinished { base, .. }
            | Self::RunError { base, .. }
            | Self::StepStarted { base, .. }
            | Self::StepFinished { base, .. }
            | Self::TextMessageStart { base, .. }
            | Self::TextMessageContent { base, .. }
            | Self::TextMessageEnd { base, .. }
            | Self::TextMessageChunk { base, .. }
            | Self::ToolCallStart { base, .. }
            | Self::ToolCallArgs { base, .. }
            | Self::ToolCallEnd { base, .. }
            | Self::ToolCallResult { base, .. }
            | Self::ToolCallChunk { base, .. }
            | Self::StateSnapshot { base, .. }
            | Self::StateDelta { base, .. }
            | Self::MessagesSnapshot { base, .. }
            | Self::ActivitySnapshot { base, .. }
            | Self::ActivityDelta { base, .. }
            | Self::Raw { base, .. }
            | Self::Custom { base, .. } => {
                base.timestamp = Some(timestamp);
            }
        }
        self
    }

    /// Get current timestamp in milliseconds.
    pub fn now_millis() -> u64 {
        use std::time::{SystemTime, UNIX_EPOCH};
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0)
    }
}

// ============================================================================
// Interaction to AG-UI Conversion
// ============================================================================

impl Interaction {
    /// Convert to AG-UI tool call events.
    ///
    /// Maps the interaction to a frontend tool call sequence:
    /// - `action` becomes the tool name
    /// - `id` becomes the tool call ID
    /// - Other fields are passed as tool arguments
    ///
    /// This is a pure protocol mapping with no semantic interpretation.
    /// The client determines how to handle each action.
    pub fn to_ag_ui_events(&self) -> Vec<AGUIEvent> {
        let args = json!({
            "id": self.id,
            "message": self.message,
            "parameters": self.parameters,
            "response_schema": self.response_schema,
        });

        vec![
            AGUIEvent::tool_call_start(&self.id, &self.action, None),
            AGUIEvent::tool_call_args(&self.id, serde_json::to_string(&args).unwrap_or_default()),
            AGUIEvent::tool_call_end(&self.id),
        ]
    }
}

// ============================================================================
