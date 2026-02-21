use crate::types::Role;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use tirea_contract::Interaction;
use tracing::warn;

// Base Event Fields
// ============================================================================

/// Common fields for all AG-UI events (BaseEvent).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct BaseEvent {
    /// Event timestamp in milliseconds since epoch.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<u64>,
    /// Raw event data from external systems.
    #[serde(rename = "rawEvent", skip_serializing_if = "Option::is_none")]
    pub raw_event: Option<Value>,
}

/// Entity kind for `REASONING_ENCRYPTED_VALUE`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ReasoningEncryptedValueSubtype {
    ToolCall,
    Message,
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
pub enum Event {
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
        base: BaseEvent,
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
        base: BaseEvent,
    },

    /// Indicates an error occurred during the run.
    #[serde(rename = "RUN_ERROR")]
    RunError {
        message: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        code: Option<String>,
        #[serde(flatten)]
        base: BaseEvent,
    },

    /// Marks the beginning of a step within a run.
    #[serde(rename = "STEP_STARTED")]
    StepStarted {
        #[serde(rename = "stepName")]
        step_name: String,
        #[serde(flatten)]
        base: BaseEvent,
    },

    /// Marks the completion of a step.
    #[serde(rename = "STEP_FINISHED")]
    StepFinished {
        #[serde(rename = "stepName")]
        step_name: String,
        #[serde(flatten)]
        base: BaseEvent,
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
        role: Role,
        #[serde(flatten)]
        base: BaseEvent,
    },

    /// Contains incremental text content.
    #[serde(rename = "TEXT_MESSAGE_CONTENT")]
    TextMessageContent {
        #[serde(rename = "messageId")]
        message_id: String,
        delta: String,
        #[serde(flatten)]
        base: BaseEvent,
    },

    /// Indicates the end of a text message stream.
    #[serde(rename = "TEXT_MESSAGE_END")]
    TextMessageEnd {
        #[serde(rename = "messageId")]
        message_id: String,
        #[serde(flatten)]
        base: BaseEvent,
    },

    /// Combined chunk event for text messages (alternative to Start/Content/End).
    #[serde(rename = "TEXT_MESSAGE_CHUNK")]
    TextMessageChunk {
        #[serde(rename = "messageId", skip_serializing_if = "Option::is_none")]
        message_id: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        role: Option<Role>,
        #[serde(skip_serializing_if = "Option::is_none")]
        delta: Option<String>,
        #[serde(flatten)]
        base: BaseEvent,
    },

    // ========================================================================
    // Reasoning Events
    // ========================================================================
    /// Marks the start of a reasoning phase for a message.
    #[serde(rename = "REASONING_START")]
    ReasoningStart {
        #[serde(rename = "messageId")]
        message_id: String,
        #[serde(flatten)]
        base: BaseEvent,
    },

    /// Marks the start of a streamed reasoning message.
    #[serde(rename = "REASONING_MESSAGE_START")]
    ReasoningMessageStart {
        #[serde(rename = "messageId")]
        message_id: String,
        role: Role,
        #[serde(flatten)]
        base: BaseEvent,
    },

    /// Contains incremental reasoning text.
    #[serde(rename = "REASONING_MESSAGE_CONTENT")]
    ReasoningMessageContent {
        #[serde(rename = "messageId")]
        message_id: String,
        delta: String,
        #[serde(flatten)]
        base: BaseEvent,
    },

    /// Marks the end of a streamed reasoning message.
    #[serde(rename = "REASONING_MESSAGE_END")]
    ReasoningMessageEnd {
        #[serde(rename = "messageId")]
        message_id: String,
        #[serde(flatten)]
        base: BaseEvent,
    },

    /// Combined reasoning chunk event (alternative to Start/Content/End).
    #[serde(rename = "REASONING_MESSAGE_CHUNK")]
    ReasoningMessageChunk {
        #[serde(rename = "messageId", skip_serializing_if = "Option::is_none")]
        message_id: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        delta: Option<String>,
        #[serde(flatten)]
        base: BaseEvent,
    },

    /// Marks the end of a reasoning phase for a message.
    #[serde(rename = "REASONING_END")]
    ReasoningEnd {
        #[serde(rename = "messageId")]
        message_id: String,
        #[serde(flatten)]
        base: BaseEvent,
    },

    /// Attaches an encrypted reasoning value to a message or tool call.
    #[serde(rename = "REASONING_ENCRYPTED_VALUE")]
    ReasoningEncryptedValue {
        subtype: ReasoningEncryptedValueSubtype,
        #[serde(rename = "entityId")]
        entity_id: String,
        #[serde(rename = "encryptedValue")]
        encrypted_value: String,
        #[serde(flatten)]
        base: BaseEvent,
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
        base: BaseEvent,
    },

    /// Contains incremental tool arguments.
    #[serde(rename = "TOOL_CALL_ARGS")]
    ToolCallArgs {
        #[serde(rename = "toolCallId")]
        tool_call_id: String,
        delta: String,
        #[serde(flatten)]
        base: BaseEvent,
    },

    /// Signals the end of tool argument streaming.
    #[serde(rename = "TOOL_CALL_END")]
    ToolCallEnd {
        #[serde(rename = "toolCallId")]
        tool_call_id: String,
        #[serde(flatten)]
        base: BaseEvent,
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
        role: Option<Role>,
        #[serde(flatten)]
        base: BaseEvent,
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
        base: BaseEvent,
    },

    // ========================================================================
    // State Management Events
    // ========================================================================
    /// Provides a complete state snapshot.
    #[serde(rename = "STATE_SNAPSHOT")]
    StateSnapshot {
        snapshot: Value,
        #[serde(flatten)]
        base: BaseEvent,
    },

    /// Contains incremental state changes (RFC 6902 JSON Patch).
    #[serde(rename = "STATE_DELTA")]
    StateDelta {
        /// Array of JSON Patch operations.
        delta: Vec<Value>,
        #[serde(flatten)]
        base: BaseEvent,
    },

    /// Provides a complete message history snapshot.
    #[serde(rename = "MESSAGES_SNAPSHOT")]
    MessagesSnapshot {
        messages: Vec<Value>,
        #[serde(flatten)]
        base: BaseEvent,
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
        base: BaseEvent,
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
        base: BaseEvent,
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
        base: BaseEvent,
    },

    /// Custom application-defined event.
    #[serde(rename = "CUSTOM")]
    Custom {
        name: String,
        value: Value,
        #[serde(flatten)]
        base: BaseEvent,
    },
}

impl Event {
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
            base: BaseEvent::default(),
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
            base: BaseEvent::default(),
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
            base: BaseEvent::default(),
        }
    }

    /// Create a run-error event.
    pub fn run_error(message: impl Into<String>, code: Option<String>) -> Self {
        Self::RunError {
            message: message.into(),
            code,
            base: BaseEvent::default(),
        }
    }

    /// Create a step-started event.
    pub fn step_started(step_name: impl Into<String>) -> Self {
        Self::StepStarted {
            step_name: step_name.into(),
            base: BaseEvent::default(),
        }
    }

    /// Create a step-finished event.
    pub fn step_finished(step_name: impl Into<String>) -> Self {
        Self::StepFinished {
            step_name: step_name.into(),
            base: BaseEvent::default(),
        }
    }

    // ========================================================================
    // Factory Methods - Text Message
    // ========================================================================

    /// Create a text-message-start event.
    pub fn text_message_start(message_id: impl Into<String>) -> Self {
        Self::TextMessageStart {
            message_id: message_id.into(),
            role: Role::Assistant,
            base: BaseEvent::default(),
        }
    }

    /// Create a text-message-content event.
    pub fn text_message_content(message_id: impl Into<String>, delta: impl Into<String>) -> Self {
        Self::TextMessageContent {
            message_id: message_id.into(),
            delta: delta.into(),
            base: BaseEvent::default(),
        }
    }

    /// Create a text-message-end event.
    pub fn text_message_end(message_id: impl Into<String>) -> Self {
        Self::TextMessageEnd {
            message_id: message_id.into(),
            base: BaseEvent::default(),
        }
    }

    /// Create a text-message-chunk event.
    pub fn text_message_chunk(
        message_id: Option<String>,
        role: Option<Role>,
        delta: Option<String>,
    ) -> Self {
        Self::TextMessageChunk {
            message_id,
            role,
            delta,
            base: BaseEvent::default(),
        }
    }

    // ========================================================================
    // Factory Methods - Reasoning
    // ========================================================================

    /// Create a reasoning-start event.
    pub fn reasoning_start(message_id: impl Into<String>) -> Self {
        Self::ReasoningStart {
            message_id: message_id.into(),
            base: BaseEvent::default(),
        }
    }

    /// Create a reasoning-message-start event.
    pub fn reasoning_message_start(message_id: impl Into<String>) -> Self {
        Self::ReasoningMessageStart {
            message_id: message_id.into(),
            role: Role::Assistant,
            base: BaseEvent::default(),
        }
    }

    /// Create a reasoning-message-content event.
    pub fn reasoning_message_content(
        message_id: impl Into<String>,
        delta: impl Into<String>,
    ) -> Self {
        Self::ReasoningMessageContent {
            message_id: message_id.into(),
            delta: delta.into(),
            base: BaseEvent::default(),
        }
    }

    /// Create a reasoning-message-end event.
    pub fn reasoning_message_end(message_id: impl Into<String>) -> Self {
        Self::ReasoningMessageEnd {
            message_id: message_id.into(),
            base: BaseEvent::default(),
        }
    }

    /// Create a reasoning-message-chunk event.
    pub fn reasoning_message_chunk(message_id: Option<String>, delta: Option<String>) -> Self {
        Self::ReasoningMessageChunk {
            message_id,
            delta,
            base: BaseEvent::default(),
        }
    }

    /// Create a reasoning-end event.
    pub fn reasoning_end(message_id: impl Into<String>) -> Self {
        Self::ReasoningEnd {
            message_id: message_id.into(),
            base: BaseEvent::default(),
        }
    }

    /// Create a reasoning-encrypted-value event.
    pub fn reasoning_encrypted_value(
        subtype: ReasoningEncryptedValueSubtype,
        entity_id: impl Into<String>,
        encrypted_value: impl Into<String>,
    ) -> Self {
        Self::ReasoningEncryptedValue {
            subtype,
            entity_id: entity_id.into(),
            encrypted_value: encrypted_value.into(),
            base: BaseEvent::default(),
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
            base: BaseEvent::default(),
        }
    }

    /// Create a tool-call-args event.
    pub fn tool_call_args(tool_call_id: impl Into<String>, delta: impl Into<String>) -> Self {
        Self::ToolCallArgs {
            tool_call_id: tool_call_id.into(),
            delta: delta.into(),
            base: BaseEvent::default(),
        }
    }

    /// Create a tool-call-end event.
    pub fn tool_call_end(tool_call_id: impl Into<String>) -> Self {
        Self::ToolCallEnd {
            tool_call_id: tool_call_id.into(),
            base: BaseEvent::default(),
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
            role: Some(Role::Tool),
            base: BaseEvent::default(),
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
            base: BaseEvent::default(),
        }
    }

    // ========================================================================
    // Factory Methods - State
    // ========================================================================

    /// Create a state-snapshot event.
    pub fn state_snapshot(snapshot: Value) -> Self {
        Self::StateSnapshot {
            snapshot,
            base: BaseEvent::default(),
        }
    }

    /// Create a state-delta event.
    pub fn state_delta(delta: Vec<Value>) -> Self {
        Self::StateDelta {
            delta,
            base: BaseEvent::default(),
        }
    }

    /// Create a messages-snapshot event.
    pub fn messages_snapshot(messages: Vec<Value>) -> Self {
        Self::MessagesSnapshot {
            messages,
            base: BaseEvent::default(),
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
            base: BaseEvent::default(),
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
            base: BaseEvent::default(),
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
            base: BaseEvent::default(),
        }
    }

    /// Create a custom event.
    pub fn custom(name: impl Into<String>, value: Value) -> Self {
        Self::Custom {
            name: name.into(),
            value,
            base: BaseEvent::default(),
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
            | Self::ReasoningStart { base, .. }
            | Self::ReasoningMessageStart { base, .. }
            | Self::ReasoningMessageContent { base, .. }
            | Self::ReasoningMessageEnd { base, .. }
            | Self::ReasoningMessageChunk { base, .. }
            | Self::ReasoningEnd { base, .. }
            | Self::ReasoningEncryptedValue { base, .. }
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

/// Convert one interaction to AG-UI tool call events.
pub fn interaction_to_ag_ui_events(interaction: &Interaction) -> Vec<Event> {
    let args = json!({
        "id": interaction.id,
        "message": interaction.message,
        "parameters": interaction.parameters,
        "response_schema": interaction.response_schema,
    });

    // Strip "tool:" prefix from action to produce the AG-UI toolCallName.
    // Internally, interactions use "tool:addTask" convention for routing,
    // but the AG-UI protocol expects the bare tool name "addTask".
    let tool_name = interaction
        .action
        .strip_prefix("tool:")
        .unwrap_or(&interaction.action);

    vec![
        Event::tool_call_start(&interaction.id, tool_name, None),
        Event::tool_call_args(
            &interaction.id,
            match serde_json::to_string(&args) {
                Ok(value) => value,
                Err(err) => {
                    warn!(
                        error = %err,
                        interaction_id = %interaction.id,
                        "failed to serialize interaction arguments for AG-UI"
                    );
                    "{}".to_string()
                }
            },
        ),
        Event::tool_call_end(&interaction.id),
    ]
}

// ============================================================================
