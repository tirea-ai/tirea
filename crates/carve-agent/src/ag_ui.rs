//! AG-UI Protocol event types.
//!
//! This module provides types for the AG-UI (Agent-User Interaction) Protocol,
//! enabling integration with CopilotKit and other AG-UI compatible frontends.
//!
//! # Protocol Overview
//!
//! AG-UI is an open, lightweight, event-based protocol that standardizes how
//! AI agents connect to user-facing applications. Events are streamed as JSON
//! over SSE, WebSocket, or other transports.
//!
//! # Event Categories
//!
//! - **Lifecycle**: RunStarted, RunFinished, RunError, StepStarted, StepFinished
//! - **Text Message**: TextMessageStart, TextMessageContent, TextMessageEnd, TextMessageChunk
//! - **Tool Call**: ToolCallStart, ToolCallArgs, ToolCallEnd, ToolCallResult, ToolCallChunk
//! - **State**: StateSnapshot, StateDelta, MessagesSnapshot
//! - **Activity**: ActivitySnapshot, ActivityDelta
//! - **Special**: Raw, Custom
//!
//! # Example
//!
//! ```rust
//! use carve_agent::ag_ui::{AGUIEvent, AGUIContext};
//! use carve_agent::AgentEvent;
//!
//! let mut ctx = AGUIContext::new("thread_1".into(), "run_1".into());
//! let event = AgentEvent::TextDelta { delta: "Hello".into() };
//! let ag_ui_events = event.to_ag_ui_events(&mut ctx);
//! ```

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

// ============================================================================
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
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum MessageRole {
    Developer,
    System,
    Assistant,
    User,
    Tool,
}

impl Default for MessageRole {
    fn default() -> Self {
        Self::Assistant
    }
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
// AG-UI Context
// ============================================================================

/// Context for AG-UI event conversion.
///
/// Maintains state needed for converting internal AgentEvents to AG-UI events.
#[derive(Debug, Clone)]
pub struct AGUIContext {
    /// Thread identifier (conversation context).
    pub thread_id: String,
    /// Current run identifier.
    pub run_id: String,
    /// Current message identifier.
    pub message_id: String,
    /// Step counter for generating step names.
    step_counter: u32,
    /// Whether text message stream has started.
    text_started: bool,
    /// Current step name.
    current_step: Option<String>,
}

impl AGUIContext {
    /// Create a new AG-UI context.
    pub fn new(thread_id: String, run_id: String) -> Self {
        let message_id = format!("msg_{}", &run_id[..8.min(run_id.len())]);
        Self {
            thread_id,
            run_id,
            message_id,
            step_counter: 0,
            text_started: false,
            current_step: None,
        }
    }

    /// Generate the next step name.
    pub fn next_step_name(&mut self) -> String {
        self.step_counter += 1;
        let name = format!("step_{}", self.step_counter);
        self.current_step = Some(name.clone());
        name
    }

    /// Get the current step name.
    pub fn current_step_name(&self) -> String {
        self.current_step
            .clone()
            .unwrap_or_else(|| format!("step_{}", self.step_counter))
    }

    /// Mark text stream as started.
    pub fn start_text(&mut self) -> bool {
        let was_started = self.text_started;
        self.text_started = true;
        !was_started
    }

    /// Mark text stream as ended and return whether it was active.
    pub fn end_text(&mut self) -> bool {
        let was_started = self.text_started;
        self.text_started = false;
        was_started
    }

    /// Generate a new message ID.
    pub fn new_message_id(&mut self) -> String {
        self.message_id = format!(
            "msg_{}_{}",
            &self.run_id[..8.min(self.run_id.len())],
            uuid_simple()
        );
        self.message_id.clone()
    }
}

/// Generate a simple unique identifier.
fn uuid_simple() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("{:x}", nanos & 0xFFFFFFFF)
}

// ============================================================================
// AG-UI Adapter
// ============================================================================

/// Adapter for AG-UI Protocol (CopilotKit compatible).
///
/// Converts `AgentEvent` to `AGUIEvent` for CopilotKit and AG-UI frontend integration.
///
/// # Example
///
/// ```rust
/// use carve_agent::{AgentEvent, AgUiAdapter};
///
/// let mut adapter = AgUiAdapter::new("thread_1".to_string(), "run_123".to_string());
///
/// // Convert event to SSE format
/// let event = AgentEvent::TextDelta { delta: "Hello".to_string() };
/// let sse_lines = adapter.to_sse(&event);
/// assert!(sse_lines[0].contains("TEXT_MESSAGE"));
/// ```
#[derive(Debug, Clone)]
pub struct AgUiAdapter {
    /// AG-UI context for event conversion.
    ctx: AGUIContext,
}

impl AgUiAdapter {
    /// Create a new AG-UI adapter.
    pub fn new(thread_id: String, run_id: String) -> Self {
        Self {
            ctx: AGUIContext::new(thread_id, run_id),
        }
    }

    /// Get the thread ID.
    pub fn thread_id(&self) -> &str {
        &self.ctx.thread_id
    }

    /// Get the run ID.
    pub fn run_id(&self) -> &str {
        &self.ctx.run_id
    }

    /// Get the message ID.
    pub fn message_id(&self) -> &str {
        &self.ctx.message_id
    }

    /// Get mutable access to the context.
    pub fn context_mut(&mut self) -> &mut AGUIContext {
        &mut self.ctx
    }

    /// Convert an AgentEvent to AGUIEvent(s).
    pub fn convert(&mut self, event: &crate::stream::AgentEvent) -> Vec<AGUIEvent> {
        event.to_ag_ui_events(&mut self.ctx)
    }

    /// Convert an AgentEvent to JSON strings.
    pub fn to_json(&mut self, event: &crate::stream::AgentEvent) -> Vec<String> {
        self.convert(event)
            .into_iter()
            .filter_map(|e| serde_json::to_string(&e).ok())
            .collect()
    }

    /// Convert an AgentEvent to SSE (Server-Sent Events) format.
    ///
    /// Returns lines in the format: `data: {json}\n\n`
    pub fn to_sse(&mut self, event: &crate::stream::AgentEvent) -> Vec<String> {
        self.to_json(event)
            .into_iter()
            .map(|json| format!("data: {}\n\n", json))
            .collect()
    }

    /// Convert an AgentEvent to newline-delimited JSON (NDJSON) format.
    ///
    /// Returns lines in the format: `{json}\n`
    pub fn to_ndjson(&mut self, event: &crate::stream::AgentEvent) -> Vec<String> {
        self.to_json(event)
            .into_iter()
            .map(|json| format!("{}\n", json))
            .collect()
    }

    /// Generate run started event.
    pub fn run_started(&self, parent_run_id: Option<String>) -> AGUIEvent {
        AGUIEvent::run_started(&self.ctx.thread_id, &self.ctx.run_id, parent_run_id)
    }

    /// Generate run finished event.
    pub fn run_finished(&self, result: Option<serde_json::Value>) -> AGUIEvent {
        AGUIEvent::run_finished(&self.ctx.thread_id, &self.ctx.run_id, result)
    }

    /// Generate run error event.
    pub fn run_error(&self, message: impl Into<String>, code: Option<String>) -> AGUIEvent {
        AGUIEvent::run_error(message, code)
    }
}

// ============================================================================
// AG-UI Request Types
// ============================================================================

/// AG-UI message in a conversation.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AGUIMessage {
    /// Message role (user, assistant, system, tool).
    pub role: MessageRole,
    /// Message content.
    pub content: String,
    /// Optional message ID.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    /// Optional tool call ID (for tool messages).
    #[serde(rename = "toolCallId", skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
}

impl AGUIMessage {
    /// Create a user message.
    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: MessageRole::User,
            content: content.into(),
            id: None,
            tool_call_id: None,
        }
    }

    /// Create an assistant message.
    pub fn assistant(content: impl Into<String>) -> Self {
        Self {
            role: MessageRole::Assistant,
            content: content.into(),
            id: None,
            tool_call_id: None,
        }
    }

    /// Create a system message.
    pub fn system(content: impl Into<String>) -> Self {
        Self {
            role: MessageRole::System,
            content: content.into(),
            id: None,
            tool_call_id: None,
        }
    }

    /// Create a tool result message.
    pub fn tool(content: impl Into<String>, tool_call_id: impl Into<String>) -> Self {
        Self {
            role: MessageRole::Tool,
            content: content.into(),
            id: None,
            tool_call_id: Some(tool_call_id.into()),
        }
    }
}

/// Tool execution location.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum ToolExecutionLocation {
    /// Tool executes on the backend (server-side).
    #[default]
    Backend,
    /// Tool executes on the frontend (client-side).
    Frontend,
}

/// AG-UI tool definition.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AGUIToolDef {
    /// Tool name.
    pub name: String,
    /// Tool description.
    pub description: String,
    /// JSON Schema for tool parameters.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parameters: Option<Value>,
    /// Where the tool executes (frontend or backend).
    /// Frontend tools are executed by the client and results sent back.
    #[serde(default, skip_serializing_if = "is_default_backend")]
    pub execute: ToolExecutionLocation,
}

fn is_default_backend(loc: &ToolExecutionLocation) -> bool {
    *loc == ToolExecutionLocation::Backend
}

impl AGUIToolDef {
    /// Create a new backend tool definition.
    pub fn backend(name: impl Into<String>, description: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            description: description.into(),
            parameters: None,
            execute: ToolExecutionLocation::Backend,
        }
    }

    /// Create a new frontend tool definition.
    pub fn frontend(name: impl Into<String>, description: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            description: description.into(),
            parameters: None,
            execute: ToolExecutionLocation::Frontend,
        }
    }

    /// Set the JSON Schema parameters.
    pub fn with_parameters(mut self, parameters: Value) -> Self {
        self.parameters = Some(parameters);
        self
    }

    /// Check if this is a frontend tool.
    pub fn is_frontend(&self) -> bool {
        self.execute == ToolExecutionLocation::Frontend
    }
}

/// Request to run an AG-UI agent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunAgentRequest {
    /// Thread identifier.
    #[serde(rename = "threadId")]
    pub thread_id: String,
    /// Run identifier.
    #[serde(rename = "runId")]
    pub run_id: String,
    /// Conversation messages.
    pub messages: Vec<AGUIMessage>,
    /// Available tools.
    #[serde(default)]
    pub tools: Vec<AGUIToolDef>,
    /// Initial state.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state: Option<Value>,
    /// Parent run ID (for sub-runs).
    #[serde(rename = "parentRunId", skip_serializing_if = "Option::is_none")]
    pub parent_run_id: Option<String>,
    /// Model to use.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// System prompt.
    #[serde(rename = "systemPrompt", skip_serializing_if = "Option::is_none")]
    pub system_prompt: Option<String>,
    /// Additional configuration.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub config: Option<Value>,
}

impl RunAgentRequest {
    /// Create a new request with minimal required fields.
    pub fn new(thread_id: impl Into<String>, run_id: impl Into<String>) -> Self {
        Self {
            thread_id: thread_id.into(),
            run_id: run_id.into(),
            messages: Vec::new(),
            tools: Vec::new(),
            state: None,
            parent_run_id: None,
            model: None,
            system_prompt: None,
            config: None,
        }
    }

    /// Add a message.
    pub fn with_message(mut self, message: AGUIMessage) -> Self {
        self.messages.push(message);
        self
    }

    /// Add messages.
    pub fn with_messages(mut self, messages: Vec<AGUIMessage>) -> Self {
        self.messages.extend(messages);
        self
    }

    /// Set initial state.
    pub fn with_state(mut self, state: Value) -> Self {
        self.state = Some(state);
        self
    }

    /// Set model.
    pub fn with_model(mut self, model: impl Into<String>) -> Self {
        self.model = Some(model.into());
        self
    }

    /// Set system prompt.
    pub fn with_system_prompt(mut self, prompt: impl Into<String>) -> Self {
        self.system_prompt = Some(prompt.into());
        self
    }

    /// Validate the request.
    pub fn validate(&self) -> Result<(), RequestError> {
        if self.thread_id.is_empty() {
            return Err(RequestError::invalid_field("threadId cannot be empty"));
        }
        if self.run_id.is_empty() {
            return Err(RequestError::invalid_field("runId cannot be empty"));
        }
        Ok(())
    }

    /// Get the last user message content.
    pub fn last_user_message(&self) -> Option<&str> {
        self.messages
            .iter()
            .rev()
            .find(|m| m.role == MessageRole::User)
            .map(|m| m.content.as_str())
    }

    /// Get frontend tools from the request.
    pub fn frontend_tools(&self) -> Vec<&AGUIToolDef> {
        self.tools.iter().filter(|t| t.is_frontend()).collect()
    }

    /// Get backend tools from the request.
    pub fn backend_tools(&self) -> Vec<&AGUIToolDef> {
        self.tools.iter().filter(|t| !t.is_frontend()).collect()
    }

    /// Check if a tool is a frontend tool by name.
    pub fn is_frontend_tool(&self, name: &str) -> bool {
        self.tools
            .iter()
            .find(|t| t.name == name)
            .map(|t| t.is_frontend())
            .unwrap_or(false)
    }

    /// Add a tool definition.
    pub fn with_tool(mut self, tool: AGUIToolDef) -> Self {
        self.tools.push(tool);
        self
    }
}

/// Error type for request processing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RequestError {
    /// Error code.
    pub code: String,
    /// Error message.
    pub message: String,
}

impl RequestError {
    /// Create an invalid field error.
    pub fn invalid_field(message: impl Into<String>) -> Self {
        Self {
            code: "INVALID_FIELD".into(),
            message: message.into(),
        }
    }

    /// Create a validation error.
    pub fn validation(message: impl Into<String>) -> Self {
        Self {
            code: "VALIDATION_ERROR".into(),
            message: message.into(),
        }
    }

    /// Create an internal error.
    pub fn internal(message: impl Into<String>) -> Self {
        Self {
            code: "INTERNAL_ERROR".into(),
            message: message.into(),
        }
    }
}

impl std::fmt::Display for RequestError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "[{}] {}", self.code, self.message)
    }
}

impl std::error::Error for RequestError {}

impl From<String> for RequestError {
    fn from(message: String) -> Self {
        Self::validation(message)
    }
}

// ============================================================================
// AG-UI Run Agent Stream
// ============================================================================

use crate::r#loop::{run_loop_stream, AgentConfig};
use crate::session::Session;
use crate::stream::AgentEvent;
use crate::traits::tool::Tool;
use async_stream::stream;
use futures::{Stream, StreamExt};
use genai::Client;
use std::pin::Pin;
use std::sync::Arc;

/// Run the agent loop and return a stream of AG-UI events.
///
/// This is the main entry point for AG-UI compatible agent execution.
/// It wraps `run_loop_stream` and converts all events to AG-UI format.
///
/// # Arguments
///
/// * `client` - GenAI client for LLM API calls
/// * `config` - Agent configuration (model, system prompt, etc.)
/// * `session` - Current session with conversation history
/// * `tools` - Available tools for the agent
/// * `thread_id` - AG-UI thread identifier
/// * `run_id` - AG-UI run identifier
///
/// # Returns
///
/// A stream of `AGUIEvent` that can be serialized to SSE format.
///
/// # Example
///
/// ```ignore
/// use carve_agent::ag_ui::run_agent_stream;
///
/// let stream = run_agent_stream(
///     client,
///     config,
///     session,
///     tools,
///     "thread_123".to_string(),
///     "run_456".to_string(),
/// );
///
/// // Convert to SSE and send to client
/// while let Some(event) = stream.next().await {
///     let json = serde_json::to_string(&event)?;
///     send_sse(format!("data: {}\n\n", json));
/// }
/// ```
pub fn run_agent_stream(
    client: Client,
    config: AgentConfig,
    session: Session,
    tools: HashMap<String, Arc<dyn Tool>>,
    thread_id: String,
    run_id: String,
) -> Pin<Box<dyn Stream<Item = AGUIEvent> + Send>> {
    Box::pin(stream! {
        // Create context
        let mut ctx = AGUIContext::new(thread_id.clone(), run_id.clone());

        // Emit RUN_STARTED
        yield AGUIEvent::run_started(&thread_id, &run_id, None);

        // Run the agent loop
        let mut inner_stream = run_loop_stream(client, config, session, tools);
        let mut final_response = String::new();

        while let Some(event) = inner_stream.next().await {
            // Track final response and errors
            match &event {
                AgentEvent::Done { response } => {
                    final_response = response.clone();
                }
                AgentEvent::Error { message } => {
                    yield AGUIEvent::run_error(message.clone(), None);
                    return; // Early return on error, no RUN_FINISHED
                }
                _ => {}
            }

            // Convert and emit AG-UI events
            let ag_ui_events = event.to_ag_ui_events(&mut ctx);
            for ag_event in ag_ui_events {
                yield ag_event;
            }
        }

        // Emit RUN_FINISHED (reached here means no error)
        let result = if final_response.is_empty() {
            None
        } else {
            Some(serde_json::json!({ "response": final_response }))
        };
        yield AGUIEvent::run_finished(&thread_id, &run_id, result);
    })
}

/// Run the agent loop and return a stream of SSE-formatted strings.
///
/// This is a convenience function that wraps `run_agent_stream` and formats
/// each event as Server-Sent Events (SSE) for direct HTTP streaming.
///
/// # Returns
///
/// A stream of strings in SSE format: `data: {json}\n\n`
pub fn run_agent_stream_sse(
    client: Client,
    config: AgentConfig,
    session: Session,
    tools: HashMap<String, Arc<dyn Tool>>,
    thread_id: String,
    run_id: String,
) -> Pin<Box<dyn Stream<Item = String> + Send>> {
    Box::pin(stream! {
        let mut inner = run_agent_stream(client, config, session, tools, thread_id, run_id);
        while let Some(event) = inner.next().await {
            if let Ok(json) = serde_json::to_string(&event) {
                yield format!("data: {}\n\n", json);
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_run_started_serialization() {
        let event = AGUIEvent::run_started("thread_1", "run_1", None);
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains(r#""type":"RUN_STARTED""#));
        assert!(json.contains(r#""threadId":"thread_1""#));
        assert!(json.contains(r#""runId":"run_1""#));
    }

    #[test]
    fn test_run_started_with_parent() {
        let event = AGUIEvent::run_started("thread_1", "run_1", Some("parent_1".to_string()));
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains(r#""parentRunId":"parent_1""#));
    }

    #[test]
    fn test_run_started_with_input() {
        let event = AGUIEvent::run_started_with_input(
            "thread_1",
            "run_1",
            None,
            json!({"messages": []}),
        );
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains(r#""input""#));
    }

    #[test]
    fn test_run_finished_serialization() {
        let event = AGUIEvent::run_finished("thread_1", "run_1", Some(json!({"answer": 42})));
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains(r#""type":"RUN_FINISHED""#));
        assert!(json.contains(r#""result""#));
    }

    #[test]
    fn test_run_error_serialization() {
        let event = AGUIEvent::run_error("Something went wrong", Some("ERR_001".to_string()));
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains(r#""type":"RUN_ERROR""#));
        assert!(json.contains(r#""message":"Something went wrong""#));
        assert!(json.contains(r#""code":"ERR_001""#));
    }

    #[test]
    fn test_step_events_serialization() {
        let start = AGUIEvent::step_started("search_step");
        let finish = AGUIEvent::step_finished("search_step");

        let start_json = serde_json::to_string(&start).unwrap();
        let finish_json = serde_json::to_string(&finish).unwrap();

        assert!(start_json.contains(r#""type":"STEP_STARTED""#));
        assert!(start_json.contains(r#""stepName":"search_step""#));
        assert!(finish_json.contains(r#""type":"STEP_FINISHED""#));
    }

    #[test]
    fn test_text_message_events_serialization() {
        let start = AGUIEvent::text_message_start("msg_1");
        let content = AGUIEvent::text_message_content("msg_1", "Hello ");
        let end = AGUIEvent::text_message_end("msg_1");

        let start_json = serde_json::to_string(&start).unwrap();
        assert!(start_json.contains(r#""type":"TEXT_MESSAGE_START""#));
        assert!(start_json.contains(r#""role":"assistant""#));

        assert!(serde_json::to_string(&content)
            .unwrap()
            .contains(r#""delta":"Hello ""#));
        assert!(serde_json::to_string(&end)
            .unwrap()
            .contains(r#""type":"TEXT_MESSAGE_END""#));
    }

    #[test]
    fn test_text_message_chunk_serialization() {
        let chunk = AGUIEvent::text_message_chunk(
            Some("msg_1".to_string()),
            Some(MessageRole::Assistant),
            Some("Hello".to_string()),
        );
        let json = serde_json::to_string(&chunk).unwrap();
        assert!(json.contains(r#""type":"TEXT_MESSAGE_CHUNK""#));
        assert!(json.contains(r#""messageId":"msg_1""#));
        assert!(json.contains(r#""role":"assistant""#));
        assert!(json.contains(r#""delta":"Hello""#));
    }

    #[test]
    fn test_tool_call_events_serialization() {
        let start = AGUIEvent::tool_call_start("call_1", "search", Some("msg_1".to_string()));
        let args = AGUIEvent::tool_call_args("call_1", r#"{"query":"rust"}"#);
        let end = AGUIEvent::tool_call_end("call_1");
        let result = AGUIEvent::tool_call_result("result_1", "call_1", r#"{"found":3}"#);

        assert!(serde_json::to_string(&start)
            .unwrap()
            .contains(r#""type":"TOOL_CALL_START""#));
        assert!(serde_json::to_string(&args)
            .unwrap()
            .contains(r#""type":"TOOL_CALL_ARGS""#));
        assert!(serde_json::to_string(&end)
            .unwrap()
            .contains(r#""type":"TOOL_CALL_END""#));

        let result_json = serde_json::to_string(&result).unwrap();
        assert!(result_json.contains(r#""type":"TOOL_CALL_RESULT""#));
        assert!(result_json.contains(r#""role":"tool""#));
    }

    #[test]
    fn test_tool_call_chunk_serialization() {
        let chunk = AGUIEvent::tool_call_chunk(
            Some("call_1".to_string()),
            Some("search".to_string()),
            Some("msg_1".to_string()),
            Some(r#"{"query"}"#.to_string()),
        );
        let json = serde_json::to_string(&chunk).unwrap();
        assert!(json.contains(r#""type":"TOOL_CALL_CHUNK""#));
        assert!(json.contains(r#""toolCallId":"call_1""#));
        assert!(json.contains(r#""toolCallName":"search""#));
    }

    #[test]
    fn test_state_snapshot_serialization() {
        let event = AGUIEvent::state_snapshot(json!({"user": {"name": "Alice"}}));
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains(r#""type":"STATE_SNAPSHOT""#));
        assert!(json.contains(r#""snapshot""#));
    }

    #[test]
    fn test_state_delta_serialization() {
        let event = AGUIEvent::state_delta(vec![
            json!({"op": "replace", "path": "/user/name", "value": "Bob"}),
        ]);
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains(r#""type":"STATE_DELTA""#));
        assert!(json.contains(r#""delta""#));
        assert!(json.contains(r#""op":"replace""#));
    }

    #[test]
    fn test_messages_snapshot_serialization() {
        let event = AGUIEvent::messages_snapshot(vec![
            json!({"role": "user", "content": "Hello"}),
            json!({"role": "assistant", "content": "Hi!"}),
        ]);
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains(r#""type":"MESSAGES_SNAPSHOT""#));
        assert!(json.contains(r#""messages""#));
    }

    #[test]
    fn test_activity_snapshot_serialization() {
        let mut content = HashMap::new();
        content.insert("progress".to_string(), json!(50));
        content.insert("status".to_string(), json!("running"));

        let event = AGUIEvent::activity_snapshot("msg_1", "progress", content, Some(true));
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains(r#""type":"ACTIVITY_SNAPSHOT""#));
        assert!(json.contains(r#""messageId":"msg_1""#));
        assert!(json.contains(r#""activityType":"progress""#));
        assert!(json.contains(r#""replace":true"#));
    }

    #[test]
    fn test_activity_delta_serialization() {
        let event = AGUIEvent::activity_delta(
            "msg_1",
            "progress",
            vec![json!({"op": "replace", "path": "/progress", "value": 75})],
        );
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains(r#""type":"ACTIVITY_DELTA""#));
        assert!(json.contains(r#""patch""#));
    }

    #[test]
    fn test_raw_event_serialization() {
        let event = AGUIEvent::raw(json!({"custom": "data"}), Some("external_system".to_string()));
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains(r#""type":"RAW""#));
        assert!(json.contains(r#""source":"external_system""#));
    }

    #[test]
    fn test_custom_event_serialization() {
        let event = AGUIEvent::custom("my_event", json!({"key": "value"}));
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains(r#""type":"CUSTOM""#));
        assert!(json.contains(r#""name":"my_event""#));
    }

    #[test]
    fn test_event_with_timestamp() {
        let event = AGUIEvent::text_message_start("msg_1").with_timestamp(1234567890);
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains(r#""timestamp":1234567890"#));
    }

    #[test]
    fn test_ag_ui_context_new() {
        let ctx = AGUIContext::new("thread_1".to_string(), "run_12345678".to_string());
        assert_eq!(ctx.thread_id, "thread_1");
        assert_eq!(ctx.run_id, "run_12345678");
        assert!(ctx.message_id.starts_with("msg_run_1234"));
    }

    #[test]
    fn test_ag_ui_context_step_names() {
        let mut ctx = AGUIContext::new("t".to_string(), "r".to_string());
        assert_eq!(ctx.next_step_name(), "step_1");
        assert_eq!(ctx.next_step_name(), "step_2");
        assert_eq!(ctx.current_step_name(), "step_2");
    }

    #[test]
    fn test_ag_ui_context_text_tracking() {
        let mut ctx = AGUIContext::new("t".to_string(), "r".to_string());
        assert!(ctx.start_text());
        assert!(!ctx.start_text());
        assert!(ctx.end_text());
        assert!(!ctx.end_text());
    }

    #[test]
    fn test_ag_ui_context_new_message_id() {
        let mut ctx = AGUIContext::new("t".to_string(), "run_12345678".to_string());
        let id1 = ctx.message_id.clone();
        let id2 = ctx.new_message_id();
        assert_ne!(id1, id2);
        assert!(id2.starts_with("msg_run_1234"));
    }

    #[test]
    fn test_event_deserialization() {
        let json = r#"{"type":"RUN_STARTED","threadId":"t1","runId":"r1"}"#;
        let event: AGUIEvent = serde_json::from_str(json).unwrap();
        assert!(matches!(event, AGUIEvent::RunStarted { .. }));
    }

    #[test]
    fn test_message_role_serialization() {
        assert_eq!(
            serde_json::to_string(&MessageRole::Assistant).unwrap(),
            r#""assistant""#
        );
        assert_eq!(
            serde_json::to_string(&MessageRole::User).unwrap(),
            r#""user""#
        );
        assert_eq!(
            serde_json::to_string(&MessageRole::Tool).unwrap(),
            r#""tool""#
        );
    }

    #[test]
    fn test_ag_ui_adapter() {
        use crate::stream::AgentEvent;

        let mut adapter = AgUiAdapter::new("thread_1".to_string(), "run_123".to_string());
        assert_eq!(adapter.thread_id(), "thread_1");
        assert_eq!(adapter.run_id(), "run_123");

        let event = AgentEvent::TextDelta {
            delta: "Hello".to_string(),
        };
        let outputs = adapter.to_json(&event);
        let combined = outputs.join("");
        assert!(combined.contains("TEXT_MESSAGE"));
    }

    // ========================================================================
    // AG-UI Adapter Comprehensive Flow Tests
    // ========================================================================

    #[test]
    fn test_ag_ui_adapter_full_text_flow() {
        use crate::stream::AgentEvent;

        let mut adapter = AgUiAdapter::new("thread_1".to_string(), "run_123".to_string());

        // First text delta should emit START + CONTENT
        let event1 = AgentEvent::TextDelta {
            delta: "Hello".to_string(),
        };
        let outputs1 = adapter.convert(&event1);
        assert_eq!(outputs1.len(), 2);
        assert!(matches!(outputs1[0], AGUIEvent::TextMessageStart { .. }));
        assert!(matches!(outputs1[1], AGUIEvent::TextMessageContent { .. }));

        // Second text delta should only emit CONTENT
        let event2 = AgentEvent::TextDelta {
            delta: " World".to_string(),
        };
        let outputs2 = adapter.convert(&event2);
        assert_eq!(outputs2.len(), 1);
        assert!(matches!(outputs2[0], AGUIEvent::TextMessageContent { .. }));

        // Done event should emit END
        let event3 = AgentEvent::Done {
            response: "Hello World".to_string(),
        };
        let outputs3 = adapter.convert(&event3);
        assert!(outputs3.iter().any(|e| matches!(e, AGUIEvent::TextMessageEnd { .. })));
    }

    #[test]
    fn test_ag_ui_adapter_tool_call_flow() {
        use crate::stream::AgentEvent;
        use crate::ToolResult;

        let mut adapter = AgUiAdapter::new("thread_1".to_string(), "run_123".to_string());

        // Tool call start
        let event1 = AgentEvent::ToolCallStart {
            id: "call_1".to_string(),
            name: "search".to_string(),
        };
        let outputs1 = adapter.convert(&event1);
        assert_eq!(outputs1.len(), 1);
        assert!(matches!(outputs1[0], AGUIEvent::ToolCallStart { .. }));
        if let AGUIEvent::ToolCallStart { tool_call_name, .. } = &outputs1[0] {
            assert_eq!(tool_call_name, "search");
        }

        // Tool call delta (args streaming)
        let event2 = AgentEvent::ToolCallDelta {
            id: "call_1".to_string(),
            args_delta: r#"{"query":"rust"}"#.to_string(),
        };
        let outputs2 = adapter.convert(&event2);
        assert_eq!(outputs2.len(), 1);
        assert!(matches!(outputs2[0], AGUIEvent::ToolCallArgs { .. }));

        // Tool call ready (signals tool args complete)
        let event3 = AgentEvent::ToolCallReady {
            id: "call_1".to_string(),
            name: "search".to_string(),
            arguments: json!({"query": "rust"}),
        };
        let outputs3 = adapter.convert(&event3);
        assert_eq!(outputs3.len(), 1);
        assert!(matches!(outputs3[0], AGUIEvent::ToolCallEnd { .. }));

        // Tool call done (returns result)
        let event4 = AgentEvent::ToolCallDone {
            id: "call_1".to_string(),
            result: ToolResult::success("search", json!({"count": 10})),
            patch: None,
        };
        let outputs4 = adapter.convert(&event4);
        assert_eq!(outputs4.len(), 1);
        assert!(matches!(outputs4[0], AGUIEvent::ToolCallResult { .. }));
    }

    #[test]
    fn test_ag_ui_adapter_state_events() {
        use crate::stream::AgentEvent;

        let mut adapter = AgUiAdapter::new("thread_1".to_string(), "run_123".to_string());

        // State snapshot
        let snapshot_event = AgentEvent::StateSnapshot {
            snapshot: json!({"user": "Alice"}),
        };
        let outputs = adapter.convert(&snapshot_event);
        assert_eq!(outputs.len(), 1);
        assert!(matches!(outputs[0], AGUIEvent::StateSnapshot { .. }));

        // State delta
        let delta_event = AgentEvent::StateDelta {
            delta: vec![json!({"op": "replace", "path": "/user", "value": "Bob"})],
        };
        let outputs = adapter.convert(&delta_event);
        assert_eq!(outputs.len(), 1);
        assert!(matches!(outputs[0], AGUIEvent::StateDelta { .. }));
    }

    #[test]
    fn test_ag_ui_adapter_error_event() {
        use crate::stream::AgentEvent;

        let mut adapter = AgUiAdapter::new("thread_1".to_string(), "run_123".to_string());

        let event = AgentEvent::Error {
            message: "Connection timeout".to_string(),
        };
        let outputs = adapter.convert(&event);
        // Error event emits RUN_ERROR
        assert_eq!(outputs.len(), 1);
        assert!(matches!(outputs[0], AGUIEvent::RunError { .. }));
        if let AGUIEvent::RunError { message, .. } = &outputs[0] {
            assert_eq!(message, "Connection timeout");
        }
    }

    #[test]
    fn test_ag_ui_adapter_turn_done() {
        use crate::stream::AgentEvent;
        use crate::ToolCall;

        let mut adapter = AgUiAdapter::new("thread_1".to_string(), "run_123".to_string());

        // Turn done without tool calls (just text)
        let event1 = AgentEvent::TurnDone {
            text: "Hello".to_string(),
            tool_calls: vec![],
        };
        let outputs1 = adapter.convert(&event1);
        // Should emit TEXT_MESSAGE_END if text was streaming
        // But since we didn't start text streaming, it might be empty
        assert!(outputs1.is_empty() || outputs1.iter().any(|e| matches!(e, AGUIEvent::TextMessageEnd { .. })));

        // Turn done with tool calls
        let event2 = AgentEvent::TurnDone {
            text: "".to_string(),
            tool_calls: vec![ToolCall::new("call_1", "search", json!({"q": "test"}))],
        };
        let outputs2 = adapter.convert(&event2);
        // Empty because tool calls are handled separately via ToolCallStart/Delta/Done
        assert!(outputs2.is_empty());
    }

    #[test]
    fn test_ag_ui_adapter_sse_format() {
        use crate::stream::AgentEvent;

        let mut adapter = AgUiAdapter::new("thread_1".to_string(), "run_123".to_string());

        let event = AgentEvent::TextDelta {
            delta: "Hello".to_string(),
        };
        let sse_lines = adapter.to_sse(&event);

        for line in &sse_lines {
            assert!(line.starts_with("data: "), "SSE line should start with 'data: '");
            assert!(line.ends_with("\n\n"), "SSE line should end with '\\n\\n'");
        }
    }

    #[test]
    fn test_ag_ui_adapter_ndjson_format() {
        use crate::stream::AgentEvent;

        let mut adapter = AgUiAdapter::new("thread_1".to_string(), "run_123".to_string());

        let event = AgentEvent::TextDelta {
            delta: "Hello".to_string(),
        };
        let ndjson_lines = adapter.to_ndjson(&event);

        for line in &ndjson_lines {
            assert!(!line.starts_with("data:"), "NDJSON should not have 'data:' prefix");
            assert!(line.ends_with("\n"), "NDJSON line should end with '\\n'");
            assert!(!line.ends_with("\n\n"), "NDJSON should have single newline");
        }
    }

    #[test]
    fn test_ag_ui_adapter_helper_methods() {
        let adapter = AgUiAdapter::new("thread_1".to_string(), "run_123".to_string());

        // run_started
        let event = adapter.run_started(None);
        assert!(matches!(event, AGUIEvent::RunStarted { .. }));
        if let AGUIEvent::RunStarted { thread_id, run_id, .. } = &event {
            assert_eq!(thread_id, "thread_1");
            assert_eq!(run_id, "run_123");
        }

        // run_started with parent
        let event = adapter.run_started(Some("parent_run".to_string()));
        if let AGUIEvent::RunStarted { parent_run_id, .. } = &event {
            assert_eq!(parent_run_id.as_deref(), Some("parent_run"));
        }

        // run_finished
        let event = adapter.run_finished(Some(json!({"result": "ok"})));
        assert!(matches!(event, AGUIEvent::RunFinished { .. }));

        // run_error
        let event = adapter.run_error("Something went wrong", Some("ERR_001".to_string()));
        assert!(matches!(event, AGUIEvent::RunError { .. }));
        if let AGUIEvent::RunError { message, code, .. } = &event {
            assert_eq!(message, "Something went wrong");
            assert_eq!(code.as_deref(), Some("ERR_001"));
        }
    }

    #[test]
    fn test_ag_ui_adapter_context_access() {
        let mut adapter = AgUiAdapter::new("thread_1".to_string(), "run_123".to_string());

        // Access context for advanced operations
        let ctx = adapter.context_mut();
        let step_name = ctx.next_step_name();
        assert_eq!(step_name, "step_1");

        // Generate new message ID
        let old_id = adapter.message_id().to_string();
        let new_id = adapter.context_mut().new_message_id();
        assert_ne!(old_id, new_id);
    }

    #[test]
    fn test_ag_ui_adapter_lifecycle_events() {
        use crate::stream::AgentEvent;

        let mut adapter = AgUiAdapter::new("thread_1".to_string(), "run_123".to_string());

        // Run start emits RUN_STARTED
        let event = AgentEvent::RunStart {
            thread_id: "t".to_string(),
            run_id: "r".to_string(),
            parent_run_id: None,
        };
        let outputs = adapter.convert(&event);
        assert_eq!(outputs.len(), 1);
        assert!(matches!(outputs[0], AGUIEvent::RunStarted { .. }));

        // Run finish emits RUN_FINISHED
        let event = AgentEvent::RunFinish {
            thread_id: "t".to_string(),
            run_id: "r".to_string(),
            result: Some(json!({"answer": 42})),
        };
        let outputs = adapter.convert(&event);
        assert!(outputs.iter().any(|e| matches!(e, AGUIEvent::RunFinished { .. })));
    }

    #[test]
    fn test_ag_ui_event_with_timestamp() {
        use std::time::{SystemTime, UNIX_EPOCH};
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);

        let event = AGUIEvent::run_started("t", "r", None).with_timestamp(ts);
        if let AGUIEvent::RunStarted { base, .. } = &event {
            assert!(base.timestamp.is_some());
            let stored_ts = base.timestamp.unwrap();
            // Should be a reasonable timestamp (after year 2020)
            assert!(stored_ts > 1577836800000); // 2020-01-01 in ms
            assert_eq!(stored_ts, ts);
        }
    }

    #[test]
    fn test_ag_ui_complete_session_flow() {
        use crate::stream::AgentEvent;
        use crate::ToolResult;

        let mut adapter = AgUiAdapter::new("thread_1".to_string(), "run_123".to_string());
        let mut all_events = Vec::new();

        // Simulate a complete agent session:
        // 1. User asks a question
        // 2. Agent starts thinking (text streaming)
        // 3. Agent calls a tool
        // 4. Tool returns result
        // 5. Agent responds with final answer

        // Step 1: Text delta (thinking)
        let events = adapter.convert(&AgentEvent::TextDelta {
            delta: "Let me search for that...".to_string(),
        });
        all_events.extend(events);

        // Step 2: Turn done (triggers tool call)
        let events = adapter.convert(&AgentEvent::TurnDone {
            text: "Let me search for that...".to_string(),
            tool_calls: vec![crate::ToolCall::new("call_1", "search", json!({"q": "rust"}))],
        });
        all_events.extend(events);

        // Step 3: Tool call start
        let events = adapter.convert(&AgentEvent::ToolCallStart {
            id: "call_1".to_string(),
            name: "search".to_string(),
        });
        all_events.extend(events);

        // Step 4: Tool call args
        let events = adapter.convert(&AgentEvent::ToolCallDelta {
            id: "call_1".to_string(),
            args_delta: r#"{"q":"rust"}"#.to_string(),
        });
        all_events.extend(events);

        // Step 5a: Tool call ready (args complete)
        let events = adapter.convert(&AgentEvent::ToolCallReady {
            id: "call_1".to_string(),
            name: "search".to_string(),
            arguments: json!({"q": "rust"}),
        });
        all_events.extend(events);

        // Step 5b: Tool result
        let events = adapter.convert(&AgentEvent::ToolCallDone {
            id: "call_1".to_string(),
            result: ToolResult::success("search", json!({"results": 5})),
            patch: None,
        });
        all_events.extend(events);

        // Step 6: Final response
        let events = adapter.convert(&AgentEvent::TextDelta {
            delta: "Found 5 results!".to_string(),
        });
        all_events.extend(events);

        // Step 7: Done
        let events = adapter.convert(&AgentEvent::Done {
            response: "Found 5 results!".to_string(),
        });
        all_events.extend(events);

        // Verify the flow
        assert!(!all_events.is_empty());

        // Should have text message events
        assert!(all_events.iter().any(|e| matches!(e, AGUIEvent::TextMessageStart { .. })));
        assert!(all_events.iter().any(|e| matches!(e, AGUIEvent::TextMessageContent { .. })));
        assert!(all_events.iter().any(|e| matches!(e, AGUIEvent::TextMessageEnd { .. })));

        // Should have tool call events
        assert!(all_events.iter().any(|e| matches!(e, AGUIEvent::ToolCallStart { .. })));
        assert!(all_events.iter().any(|e| matches!(e, AGUIEvent::ToolCallArgs { .. })));
        assert!(all_events.iter().any(|e| matches!(e, AGUIEvent::ToolCallEnd { .. })));
        assert!(all_events.iter().any(|e| matches!(e, AGUIEvent::ToolCallResult { .. })));
    }

    // ========================================================================
    // Request/Response Tests
    // ========================================================================

    #[test]
    fn test_ag_ui_message_creation() {
        let user_msg = AGUIMessage::user("Hello");
        assert_eq!(user_msg.role, MessageRole::User);
        assert_eq!(user_msg.content, "Hello");

        let assistant_msg = AGUIMessage::assistant("Hi there");
        assert_eq!(assistant_msg.role, MessageRole::Assistant);

        let system_msg = AGUIMessage::system("You are helpful");
        assert_eq!(system_msg.role, MessageRole::System);

        let tool_msg = AGUIMessage::tool(r#"{"result": 42}"#, "call_1");
        assert_eq!(tool_msg.role, MessageRole::Tool);
        assert_eq!(tool_msg.tool_call_id.as_deref(), Some("call_1"));
    }

    #[test]
    fn test_ag_ui_message_serialization() {
        let msg = AGUIMessage::user("Hello");
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains(r#""role":"user""#));
        assert!(json.contains(r#""content":"Hello""#));

        // Tool message with tool_call_id
        let tool_msg = AGUIMessage::tool("result", "call_123");
        let json = serde_json::to_string(&tool_msg).unwrap();
        assert!(json.contains(r#""role":"tool""#));
        assert!(json.contains(r#""toolCallId":"call_123""#));
    }

    #[test]
    fn test_ag_ui_message_deserialization() {
        let json = r#"{"role":"user","content":"Hello"}"#;
        let msg: AGUIMessage = serde_json::from_str(json).unwrap();
        assert_eq!(msg.role, MessageRole::User);
        assert_eq!(msg.content, "Hello");

        let json = r#"{"role":"tool","content":"result","toolCallId":"call_1"}"#;
        let msg: AGUIMessage = serde_json::from_str(json).unwrap();
        assert_eq!(msg.role, MessageRole::Tool);
        assert_eq!(msg.tool_call_id.as_deref(), Some("call_1"));
    }

    #[test]
    fn test_run_agent_request_creation() {
        let request = RunAgentRequest::new("thread_1", "run_1")
            .with_message(AGUIMessage::user("Hello"))
            .with_model("gpt-4")
            .with_system_prompt("You are helpful");

        assert_eq!(request.thread_id, "thread_1");
        assert_eq!(request.run_id, "run_1");
        assert_eq!(request.messages.len(), 1);
        assert_eq!(request.model.as_deref(), Some("gpt-4"));
        assert_eq!(request.system_prompt.as_deref(), Some("You are helpful"));
    }

    #[test]
    fn test_run_agent_request_validation_success() {
        let request = RunAgentRequest::new("thread_1", "run_1");
        assert!(request.validate().is_ok());
    }

    #[test]
    fn test_run_agent_request_validation_empty_thread_id() {
        let request = RunAgentRequest::new("", "run_1");
        let result = request.validate();
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert_eq!(err.code, "INVALID_FIELD");
        assert!(err.message.contains("threadId"));
    }

    #[test]
    fn test_run_agent_request_validation_empty_run_id() {
        let request = RunAgentRequest::new("thread_1", "");
        let result = request.validate();
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.message.contains("runId"));
    }

    #[test]
    fn test_run_agent_request_with_state() {
        let request = RunAgentRequest::new("t", "r")
            .with_state(json!({"counter": 0}));
        assert_eq!(request.state, Some(json!({"counter": 0})));
    }

    #[test]
    fn test_run_agent_request_with_messages() {
        let messages = vec![
            AGUIMessage::system("Be helpful"),
            AGUIMessage::user("Hello"),
            AGUIMessage::assistant("Hi!"),
        ];
        let request = RunAgentRequest::new("t", "r")
            .with_messages(messages);
        assert_eq!(request.messages.len(), 3);
    }

    #[test]
    fn test_run_agent_request_last_user_message() {
        let request = RunAgentRequest::new("t", "r")
            .with_message(AGUIMessage::user("First"))
            .with_message(AGUIMessage::assistant("Response"))
            .with_message(AGUIMessage::user("Second"));

        assert_eq!(request.last_user_message(), Some("Second"));
    }

    #[test]
    fn test_run_agent_request_last_user_message_none() {
        let request = RunAgentRequest::new("t", "r")
            .with_message(AGUIMessage::assistant("No user message"));

        assert_eq!(request.last_user_message(), None);
    }

    #[test]
    fn test_run_agent_request_serialization() {
        let request = RunAgentRequest::new("thread_1", "run_1")
            .with_message(AGUIMessage::user("Hello"))
            .with_model("gpt-4");

        let json = serde_json::to_string(&request).unwrap();
        assert!(json.contains(r#""threadId":"thread_1""#));
        assert!(json.contains(r#""runId":"run_1""#));
        assert!(json.contains(r#""messages""#));
        assert!(json.contains(r#""model":"gpt-4""#));
    }

    #[test]
    fn test_run_agent_request_deserialization() {
        let json = r#"{
            "threadId": "t1",
            "runId": "r1",
            "messages": [{"role": "user", "content": "Hello"}],
            "model": "gpt-4"
        }"#;

        let request: RunAgentRequest = serde_json::from_str(json).unwrap();
        assert_eq!(request.thread_id, "t1");
        assert_eq!(request.run_id, "r1");
        assert_eq!(request.messages.len(), 1);
        assert_eq!(request.model.as_deref(), Some("gpt-4"));
    }

    #[test]
    fn test_run_agent_request_deserialization_minimal() {
        // Minimal request with only required fields
        let json = r#"{"threadId": "t1", "runId": "r1", "messages": []}"#;

        let request: RunAgentRequest = serde_json::from_str(json).unwrap();
        assert_eq!(request.thread_id, "t1");
        assert_eq!(request.run_id, "r1");
        assert!(request.messages.is_empty());
        assert!(request.tools.is_empty());
        assert!(request.state.is_none());
    }

    #[test]
    fn test_request_error_types() {
        let err = RequestError::invalid_field("bad field");
        assert_eq!(err.code, "INVALID_FIELD");

        let err = RequestError::validation("validation failed");
        assert_eq!(err.code, "VALIDATION_ERROR");

        let err = RequestError::internal("internal error");
        assert_eq!(err.code, "INTERNAL_ERROR");
    }

    #[test]
    fn test_request_error_display() {
        let err = RequestError::invalid_field("threadId is required");
        let display = format!("{}", err);
        assert!(display.contains("INVALID_FIELD"));
        assert!(display.contains("threadId is required"));
    }

    #[test]
    fn test_request_error_from_string() {
        let err: RequestError = "some error".to_string().into();
        assert_eq!(err.code, "VALIDATION_ERROR");
        assert_eq!(err.message, "some error");
    }

    #[test]
    fn test_ag_ui_tool_def_serialization() {
        let tool = AGUIToolDef::backend("search", "Search the web")
            .with_parameters(json!({
                "type": "object",
                "properties": {
                    "query": {"type": "string"}
                }
            }));

        let json = serde_json::to_string(&tool).unwrap();
        assert!(json.contains(r#""name":"search""#));
        assert!(json.contains(r#""description":"Search the web""#));
        assert!(json.contains(r#""parameters""#));
    }

    #[test]
    fn test_run_agent_request_with_tools() {
        let json = r#"{
            "threadId": "t1",
            "runId": "r1",
            "messages": [],
            "tools": [
                {"name": "search", "description": "Search"},
                {"name": "calc", "description": "Calculate"}
            ]
        }"#;

        let request: RunAgentRequest = serde_json::from_str(json).unwrap();
        assert_eq!(request.tools.len(), 2);
        assert_eq!(request.tools[0].name, "search");
        assert_eq!(request.tools[1].name, "calc");
    }

    #[test]
    fn test_run_agent_request_with_parent_run() {
        let json = r#"{
            "threadId": "t1",
            "runId": "r1",
            "messages": [],
            "parentRunId": "parent_123"
        }"#;

        let request: RunAgentRequest = serde_json::from_str(json).unwrap();
        assert_eq!(request.parent_run_id.as_deref(), Some("parent_123"));
    }

    // ========================================================================
    // Event Flow Scenarios
    // ========================================================================

    #[test]
    fn test_scenario_simple_text_response() {
        use crate::stream::AgentEvent;

        let mut adapter = AgUiAdapter::new("t".to_string(), "r".to_string());
        let mut events = Vec::new();

        // Simulate: User asks -> Agent responds with text
        events.extend(adapter.convert(&AgentEvent::TextDelta {
            delta: "Hello, ".to_string(),
        }));
        events.extend(adapter.convert(&AgentEvent::TextDelta {
            delta: "how can I help?".to_string(),
        }));
        events.extend(adapter.convert(&AgentEvent::Done {
            response: "Hello, how can I help?".to_string(),
        }));

        // Should have: START, CONTENT, CONTENT, END, RUN_FINISHED
        assert!(events.iter().any(|e| matches!(e, AGUIEvent::TextMessageStart { .. })));
        assert_eq!(events.iter().filter(|e| matches!(e, AGUIEvent::TextMessageContent { .. })).count(), 2);
        assert!(events.iter().any(|e| matches!(e, AGUIEvent::TextMessageEnd { .. })));
        assert!(events.iter().any(|e| matches!(e, AGUIEvent::RunFinished { .. })));
    }

    #[test]
    fn test_scenario_tool_call_and_response() {
        use crate::stream::AgentEvent;
        use crate::ToolResult;

        let mut adapter = AgUiAdapter::new("t".to_string(), "r".to_string());
        let mut events = Vec::new();

        // Agent calls tool
        events.extend(adapter.convert(&AgentEvent::ToolCallStart {
            id: "call_1".to_string(),
            name: "search".to_string(),
        }));
        events.extend(adapter.convert(&AgentEvent::ToolCallDelta {
            id: "call_1".to_string(),
            args_delta: r#"{"q":"rust"}"#.to_string(),
        }));
        events.extend(adapter.convert(&AgentEvent::ToolCallReady {
            id: "call_1".to_string(),
            name: "search".to_string(),
            arguments: json!({"q": "rust"}),
        }));
        events.extend(adapter.convert(&AgentEvent::ToolCallDone {
            id: "call_1".to_string(),
            result: ToolResult::success("search", json!({"count": 5})),
            patch: None,
        }));

        // Should have tool call events
        assert!(events.iter().any(|e| matches!(e, AGUIEvent::ToolCallStart { .. })));
        assert!(events.iter().any(|e| matches!(e, AGUIEvent::ToolCallArgs { .. })));
        assert!(events.iter().any(|e| matches!(e, AGUIEvent::ToolCallEnd { .. })));
        assert!(events.iter().any(|e| matches!(e, AGUIEvent::ToolCallResult { .. })));
    }

    #[test]
    fn test_scenario_error_during_execution() {
        use crate::stream::AgentEvent;

        let mut adapter = AgUiAdapter::new("t".to_string(), "r".to_string());

        // Start text, then error
        let events1 = adapter.convert(&AgentEvent::TextDelta {
            delta: "Processing...".to_string(),
        });
        assert_eq!(events1.len(), 2); // START + CONTENT

        let events2 = adapter.convert(&AgentEvent::Error {
            message: "Connection failed".to_string(),
        });
        assert_eq!(events2.len(), 1);
        assert!(matches!(events2[0], AGUIEvent::RunError { .. }));

        if let AGUIEvent::RunError { message, .. } = &events2[0] {
            assert_eq!(message, "Connection failed");
        }
    }

    #[test]
    fn test_scenario_state_updates() {
        use crate::stream::AgentEvent;

        let mut adapter = AgUiAdapter::new("t".to_string(), "r".to_string());

        // State snapshot
        let events = adapter.convert(&AgentEvent::StateSnapshot {
            snapshot: json!({"user": "Alice", "count": 0}),
        });
        assert_eq!(events.len(), 1);
        if let AGUIEvent::StateSnapshot { snapshot, .. } = &events[0] {
            assert_eq!(snapshot["user"], "Alice");
        }

        // State delta
        let events = adapter.convert(&AgentEvent::StateDelta {
            delta: vec![json!({"op": "replace", "path": "/count", "value": 1})],
        });
        assert_eq!(events.len(), 1);
        assert!(matches!(events[0], AGUIEvent::StateDelta { .. }));
    }

    #[test]
    fn test_scenario_multi_turn_conversation() {
        use crate::stream::AgentEvent;
        use crate::ToolResult;

        let mut adapter = AgUiAdapter::new("t".to_string(), "r".to_string());
        let mut total_events = 0;

        // Turn 1: Text response
        let events = adapter.convert(&AgentEvent::TextDelta { delta: "Turn 1".to_string() });
        total_events += events.len();
        let events = adapter.convert(&AgentEvent::TurnDone {
            text: "Turn 1".to_string(),
            tool_calls: vec![crate::ToolCall::new("c1", "tool", json!({}))],
        });
        total_events += events.len();

        // Tool execution
        let events = adapter.convert(&AgentEvent::ToolCallStart { id: "c1".to_string(), name: "tool".to_string() });
        total_events += events.len();
        let events = adapter.convert(&AgentEvent::ToolCallReady { id: "c1".to_string(), name: "tool".to_string(), arguments: json!({}) });
        total_events += events.len();
        let events = adapter.convert(&AgentEvent::ToolCallDone {
            id: "c1".to_string(),
            result: ToolResult::success("tool", json!({})),
            patch: None,
        });
        total_events += events.len();

        // Turn 2: Final response
        let events = adapter.convert(&AgentEvent::TextDelta { delta: "Turn 2".to_string() });
        total_events += events.len();
        let events = adapter.convert(&AgentEvent::Done { response: "Turn 2".to_string() });
        total_events += events.len();

        // Should have multiple events
        assert!(total_events > 5);
    }

    #[test]
    fn test_scenario_abort() {
        use crate::stream::AgentEvent;

        let mut adapter = AgUiAdapter::new("t".to_string(), "r".to_string());

        let events = adapter.convert(&AgentEvent::Aborted {
            reason: "User cancelled".to_string(),
        });

        assert_eq!(events.len(), 1);
        if let AGUIEvent::RunError { message, code, .. } = &events[0] {
            assert_eq!(message, "User cancelled");
            assert_eq!(code.as_deref(), Some("ABORTED"));
        }
    }

    #[test]
    fn test_scenario_step_events() {
        use crate::stream::AgentEvent;

        let mut adapter = AgUiAdapter::new("t".to_string(), "r".to_string());

        let events1 = adapter.convert(&AgentEvent::StepStart);
        assert_eq!(events1.len(), 1);
        if let AGUIEvent::StepStarted { step_name, .. } = &events1[0] {
            assert_eq!(step_name, "step_1");
        }

        let events2 = adapter.convert(&AgentEvent::StepEnd);
        assert_eq!(events2.len(), 1);
        if let AGUIEvent::StepFinished { step_name, .. } = &events2[0] {
            assert_eq!(step_name, "step_1");
        }

        // Next step should be step_2
        let events3 = adapter.convert(&AgentEvent::StepStart);
        if let AGUIEvent::StepStarted { step_name, .. } = &events3[0] {
            assert_eq!(step_name, "step_2");
        }
    }

    #[test]
    fn test_scenario_messages_snapshot() {
        use crate::stream::AgentEvent;

        let mut adapter = AgUiAdapter::new("t".to_string(), "r".to_string());

        let events = adapter.convert(&AgentEvent::MessagesSnapshot {
            messages: vec![
                json!({"role": "user", "content": "Hi"}),
                json!({"role": "assistant", "content": "Hello"}),
            ],
        });

        assert_eq!(events.len(), 1);
        if let AGUIEvent::MessagesSnapshot { messages, .. } = &events[0] {
            assert_eq!(messages.len(), 2);
        }
    }

    // ========================================================================
    // Edge Cases Tests
    // ========================================================================

    #[test]
    fn test_edge_case_empty_text_delta() {
        use crate::stream::AgentEvent;

        let mut adapter = AgUiAdapter::new("t".to_string(), "r".to_string());

        // Empty delta should still emit events
        let events = adapter.convert(&AgentEvent::TextDelta {
            delta: "".to_string(),
        });
        // Should have START + CONTENT (even if content is empty)
        assert_eq!(events.len(), 2);
        if let AGUIEvent::TextMessageContent { delta, .. } = &events[1] {
            assert_eq!(delta, "");
        }
    }

    #[test]
    fn test_edge_case_unicode_and_emoji() {
        use crate::stream::AgentEvent;

        let mut adapter = AgUiAdapter::new("t".to_string(), "r".to_string());

        let text = "Hello 你好 🎉 مرحبا";
        let events = adapter.convert(&AgentEvent::TextDelta {
            delta: text.to_string(),
        });

        if let AGUIEvent::TextMessageContent { delta, .. } = &events[1] {
            assert_eq!(delta, text);
        }

        // Verify JSON serialization handles unicode
        let json = serde_json::to_string(&events[1]).unwrap();
        assert!(json.contains("你好"));
        assert!(json.contains("🎉"));
    }

    #[test]
    fn test_edge_case_special_characters() {
        use crate::stream::AgentEvent;

        let mut adapter = AgUiAdapter::new("t".to_string(), "r".to_string());

        // Text with JSON-sensitive characters
        let text = "Line1\nLine2\tTab\"Quote\\Backslash";
        let events = adapter.convert(&AgentEvent::TextDelta {
            delta: text.to_string(),
        });

        // Verify serialization handles escaping
        let json = serde_json::to_string(&events[1]).unwrap();
        assert!(json.contains("\\n")); // newline escaped
        assert!(json.contains("\\t")); // tab escaped
        assert!(json.contains("\\\"")); // quote escaped
    }

    #[test]
    fn test_edge_case_very_long_text() {
        use crate::stream::AgentEvent;

        let mut adapter = AgUiAdapter::new("t".to_string(), "r".to_string());

        // Very long text (10KB)
        let text = "x".repeat(10 * 1024);
        let events = adapter.convert(&AgentEvent::TextDelta {
            delta: text.clone(),
        });

        if let AGUIEvent::TextMessageContent { delta, .. } = &events[1] {
            assert_eq!(delta.len(), text.len());
        }
    }

    #[test]
    fn test_edge_case_multiple_parallel_tool_calls() {
        use crate::stream::AgentEvent;
        use crate::ToolResult;

        let mut adapter = AgUiAdapter::new("t".to_string(), "r".to_string());
        let mut events = Vec::new();

        // Start multiple tools
        events.extend(adapter.convert(&AgentEvent::ToolCallStart {
            id: "call_1".to_string(),
            name: "search".to_string(),
        }));
        events.extend(adapter.convert(&AgentEvent::ToolCallStart {
            id: "call_2".to_string(),
            name: "calc".to_string(),
        }));
        events.extend(adapter.convert(&AgentEvent::ToolCallStart {
            id: "call_3".to_string(),
            name: "read".to_string(),
        }));

        // All should emit TOOL_CALL_START
        let starts: Vec<_> = events.iter()
            .filter(|e| matches!(e, AGUIEvent::ToolCallStart { .. }))
            .collect();
        assert_eq!(starts.len(), 3);

        // End them in different order
        events.extend(adapter.convert(&AgentEvent::ToolCallReady {
            id: "call_2".to_string(),
            name: "calc".to_string(),
            arguments: json!({}),
        }));
        events.extend(adapter.convert(&AgentEvent::ToolCallDone {
            id: "call_2".to_string(),
            result: ToolResult::success("calc", json!(42)),
            patch: None,
        }));
        events.extend(adapter.convert(&AgentEvent::ToolCallReady {
            id: "call_1".to_string(),
            name: "search".to_string(),
            arguments: json!({}),
        }));
        events.extend(adapter.convert(&AgentEvent::ToolCallDone {
            id: "call_1".to_string(),
            result: ToolResult::success("search", json!([])),
            patch: None,
        }));

        let ends: Vec<_> = events.iter()
            .filter(|e| matches!(e, AGUIEvent::ToolCallEnd { .. }))
            .collect();
        assert_eq!(ends.len(), 2);
    }

    // ========================================================================
    // Tool Result Variants Tests
    // ========================================================================

    #[test]
    fn test_tool_result_error() {
        use crate::stream::AgentEvent;
        use crate::ToolResult;

        let mut adapter = AgUiAdapter::new("t".to_string(), "r".to_string());

        let events = adapter.convert(&AgentEvent::ToolCallDone {
            id: "call_1".to_string(),
            result: ToolResult::error("search", "Connection timeout"),
            patch: None,
        });

        assert_eq!(events.len(), 1);
        if let AGUIEvent::ToolCallResult { content, .. } = &events[0] {
            assert!(content.contains("error"));
        }
    }

    #[test]
    fn test_tool_result_pending() {
        use crate::stream::AgentEvent;
        use crate::ToolResult;

        let mut adapter = AgUiAdapter::new("t".to_string(), "r".to_string());

        let events = adapter.convert(&AgentEvent::ToolCallDone {
            id: "call_1".to_string(),
            result: ToolResult::pending("long_task", "Task is running"),
            patch: None,
        });

        assert_eq!(events.len(), 1);
        if let AGUIEvent::ToolCallResult { content, .. } = &events[0] {
            assert!(content.contains("pending"));
        }
    }

    #[test]
    fn test_tool_result_warning() {
        use crate::stream::AgentEvent;
        use crate::ToolResult;

        let mut adapter = AgUiAdapter::new("t".to_string(), "r".to_string());

        let events = adapter.convert(&AgentEvent::ToolCallDone {
            id: "call_1".to_string(),
            result: ToolResult::warning("search", json!({"partial": true}), "Rate limited"),
            patch: None,
        });

        assert_eq!(events.len(), 1);
        if let AGUIEvent::ToolCallResult { content, .. } = &events[0] {
            assert!(content.contains("warning"));
        }
    }

    // ========================================================================
    // Request Validation Edge Cases
    // ========================================================================

    #[test]
    fn test_request_with_deeply_nested_state() {
        let nested_state = json!({
            "level1": {
                "level2": {
                    "level3": {
                        "level4": {
                            "value": [1, 2, 3]
                        }
                    }
                }
            }
        });

        let request = RunAgentRequest::new("t", "r")
            .with_state(nested_state.clone());

        // Serialize and deserialize
        let json = serde_json::to_string(&request).unwrap();
        let parsed: RunAgentRequest = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.state, Some(nested_state));
    }

    #[test]
    fn test_request_with_large_messages() {
        let large_content = "x".repeat(100_000);
        let request = RunAgentRequest::new("t", "r")
            .with_message(AGUIMessage::user(&large_content));

        assert_eq!(request.messages[0].content.len(), 100_000);

        // Should serialize successfully
        let json = serde_json::to_string(&request).unwrap();
        assert!(json.len() > 100_000);
    }

    #[test]
    fn test_request_deserialization_with_extra_fields() {
        // JSON with unknown fields should be ignored
        let json = r#"{
            "threadId": "t1",
            "runId": "r1",
            "messages": [],
            "unknownField": "ignored",
            "anotherUnknown": 123
        }"#;

        let request: RunAgentRequest = serde_json::from_str(json).unwrap();
        assert_eq!(request.thread_id, "t1");
    }

    #[test]
    fn test_request_with_all_message_types() {
        let request = RunAgentRequest::new("t", "r")
            .with_message(AGUIMessage::system("System prompt"))
            .with_message(AGUIMessage::user("User input"))
            .with_message(AGUIMessage::assistant("Assistant response"))
            .with_message(AGUIMessage::tool("Tool result", "call_1"));

        assert_eq!(request.messages.len(), 4);
        assert_eq!(request.messages[0].role, MessageRole::System);
        assert_eq!(request.messages[1].role, MessageRole::User);
        assert_eq!(request.messages[2].role, MessageRole::Assistant);
        assert_eq!(request.messages[3].role, MessageRole::Tool);
    }

    // ========================================================================
    // SSE Format Edge Cases
    // ========================================================================

    #[test]
    fn test_sse_format_with_newlines_in_content() {
        use crate::stream::AgentEvent;

        let mut adapter = AgUiAdapter::new("t".to_string(), "r".to_string());

        let text_with_newlines = "Line 1\nLine 2\nLine 3";
        let event = AgentEvent::TextDelta {
            delta: text_with_newlines.to_string(),
        };

        let sse_lines = adapter.to_sse(&event);

        // Each SSE line should be properly formatted
        for line in &sse_lines {
            assert!(line.starts_with("data: "));
            assert!(line.ends_with("\n\n"));
            // The JSON inside should have escaped newlines
            let json_part = &line[6..line.len()-2];
            assert!(serde_json::from_str::<serde_json::Value>(json_part).is_ok());
        }
    }

    #[test]
    fn test_sse_format_multiple_events() {
        use crate::stream::AgentEvent;

        let mut adapter = AgUiAdapter::new("t".to_string(), "r".to_string());

        // First event (generates 2: START + CONTENT)
        let sse1 = adapter.to_sse(&AgentEvent::TextDelta {
            delta: "Hello".to_string(),
        });
        assert_eq!(sse1.len(), 2);

        // Second event (generates 1: CONTENT)
        let sse2 = adapter.to_sse(&AgentEvent::TextDelta {
            delta: " World".to_string(),
        });
        assert_eq!(sse2.len(), 1);

        // All should be valid SSE format
        for line in sse1.iter().chain(sse2.iter()) {
            assert!(line.starts_with("data: "));
            assert!(line.ends_with("\n\n"));
        }
    }

    // ========================================================================
    // BaseEvent Fields Tests
    // ========================================================================

    #[test]
    fn test_base_event_with_raw_event() {
        let mut event = AGUIEvent::run_started("t", "r", None);

        // Add raw event
        if let AGUIEvent::RunStarted { ref mut base, .. } = event {
            base.raw_event = Some(json!({"original": "data"}));
        }

        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("rawEvent"));
        assert!(json.contains("original"));
    }

    #[test]
    fn test_base_event_timestamp_precision() {
        use std::time::{SystemTime, UNIX_EPOCH};

        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);

        let event = AGUIEvent::text_message_start("msg_1").with_timestamp(ts);

        if let AGUIEvent::TextMessageStart { base, .. } = &event {
            assert_eq!(base.timestamp, Some(ts));
        }

        // Serialize and verify timestamp is preserved
        let json = serde_json::to_string(&event).unwrap();
        let parsed: AGUIEvent = serde_json::from_str(&json).unwrap();

        if let AGUIEvent::TextMessageStart { base, .. } = parsed {
            assert_eq!(base.timestamp, Some(ts));
        }
    }

    // ========================================================================
    // Event Order Consistency Tests
    // ========================================================================

    #[test]
    fn test_event_order_text_only() {
        use crate::stream::AgentEvent;

        let mut adapter = AgUiAdapter::new("t".to_string(), "r".to_string());
        let mut events = Vec::new();

        // Simulate: multiple text deltas then done
        for i in 0..5 {
            events.extend(adapter.convert(&AgentEvent::TextDelta {
                delta: format!("chunk_{}", i),
            }));
        }
        events.extend(adapter.convert(&AgentEvent::Done {
            response: "full response".to_string(),
        }));

        // Verify order: START, CONTENT*5, END, RUN_FINISHED
        assert!(matches!(events[0], AGUIEvent::TextMessageStart { .. }));
        for i in 1..=5 {
            assert!(matches!(events[i], AGUIEvent::TextMessageContent { .. }));
        }
        assert!(matches!(events[6], AGUIEvent::TextMessageEnd { .. }));
        assert!(matches!(events[7], AGUIEvent::RunFinished { .. }));
    }

    #[test]
    fn test_event_order_text_then_tool() {
        use crate::stream::AgentEvent;

        let mut adapter = AgUiAdapter::new("t".to_string(), "r".to_string());
        let mut events = Vec::new();

        // Text first
        events.extend(adapter.convert(&AgentEvent::TextDelta {
            delta: "Thinking...".to_string(),
        }));

        // Then tool call (should end text first)
        events.extend(adapter.convert(&AgentEvent::ToolCallStart {
            id: "c1".to_string(),
            name: "tool".to_string(),
        }));

        // Order: TEXT_START, TEXT_CONTENT, TEXT_END, TOOL_START
        assert!(matches!(events[0], AGUIEvent::TextMessageStart { .. }));
        assert!(matches!(events[1], AGUIEvent::TextMessageContent { .. }));
        assert!(matches!(events[2], AGUIEvent::TextMessageEnd { .. }));
        assert!(matches!(events[3], AGUIEvent::ToolCallStart { .. }));
    }

    #[test]
    fn test_context_state_consistency() {
        let mut ctx = AGUIContext::new("t".to_string(), "r".to_string());

        // Initially text not started
        assert!(ctx.start_text()); // returns true, sets started
        assert!(!ctx.start_text()); // returns false, already started
        assert!(!ctx.start_text()); // still false

        assert!(ctx.end_text()); // returns true, was active
        assert!(!ctx.end_text()); // returns false, already ended

        // Can start again
        assert!(ctx.start_text());
    }

    // ========================================================================
    // Deserialization Error Handling
    // ========================================================================

    #[test]
    fn test_invalid_message_role_deserialization() {
        let json = r#"{"role":"invalid","content":"test"}"#;
        let result: Result<AGUIMessage, _> = serde_json::from_str(json);
        assert!(result.is_err());
    }

    #[test]
    fn test_missing_required_fields() {
        // Missing content
        let json = r#"{"role":"user"}"#;
        let result: Result<AGUIMessage, _> = serde_json::from_str(json);
        assert!(result.is_err());

        // Missing threadId
        let json = r#"{"runId":"r1","messages":[]}"#;
        let result: Result<RunAgentRequest, _> = serde_json::from_str(json);
        assert!(result.is_err());
    }

    #[test]
    fn test_event_type_deserialization() {
        // All event types should deserialize correctly
        let test_cases = vec![
            r#"{"type":"RUN_STARTED","threadId":"t","runId":"r"}"#,
            r#"{"type":"RUN_FINISHED","threadId":"t","runId":"r"}"#,
            r#"{"type":"RUN_ERROR","message":"err"}"#,
            r#"{"type":"TEXT_MESSAGE_START","messageId":"m","role":"assistant"}"#,
            r#"{"type":"TEXT_MESSAGE_CONTENT","messageId":"m","delta":"x"}"#,
            r#"{"type":"TEXT_MESSAGE_END","messageId":"m"}"#,
            r#"{"type":"TOOL_CALL_START","toolCallId":"c","toolCallName":"n"}"#,
            r#"{"type":"TOOL_CALL_END","toolCallId":"c"}"#,
            r#"{"type":"STATE_SNAPSHOT","snapshot":{}}"#,
        ];

        for json in test_cases {
            let result: Result<AGUIEvent, _> = serde_json::from_str(json);
            assert!(result.is_ok(), "Failed to deserialize: {}", json);
        }
    }

    // ========================================================================
    // Frontend Tool Tests
    // ========================================================================

    #[test]
    fn test_tool_execution_location_serialization() {
        assert_eq!(
            serde_json::to_string(&ToolExecutionLocation::Backend).unwrap(),
            r#""backend""#
        );
        assert_eq!(
            serde_json::to_string(&ToolExecutionLocation::Frontend).unwrap(),
            r#""frontend""#
        );
    }

    #[test]
    fn test_tool_execution_location_deserialization() {
        let backend: ToolExecutionLocation = serde_json::from_str(r#""backend""#).unwrap();
        let frontend: ToolExecutionLocation = serde_json::from_str(r#""frontend""#).unwrap();
        assert_eq!(backend, ToolExecutionLocation::Backend);
        assert_eq!(frontend, ToolExecutionLocation::Frontend);
    }

    #[test]
    fn test_tool_execution_location_default() {
        assert_eq!(ToolExecutionLocation::default(), ToolExecutionLocation::Backend);
    }

    #[test]
    fn test_ag_ui_tool_def_backend_factory() {
        let tool = AGUIToolDef::backend("search", "Search the web");
        assert_eq!(tool.name, "search");
        assert_eq!(tool.description, "Search the web");
        assert_eq!(tool.execute, ToolExecutionLocation::Backend);
        assert!(tool.parameters.is_none());
        assert!(!tool.is_frontend());
    }

    #[test]
    fn test_ag_ui_tool_def_frontend_factory() {
        let tool = AGUIToolDef::frontend("copyToClipboard", "Copy text to clipboard");
        assert_eq!(tool.name, "copyToClipboard");
        assert_eq!(tool.description, "Copy text to clipboard");
        assert_eq!(tool.execute, ToolExecutionLocation::Frontend);
        assert!(tool.parameters.is_none());
        assert!(tool.is_frontend());
    }

    #[test]
    fn test_ag_ui_tool_def_with_parameters() {
        let tool = AGUIToolDef::frontend("showDialog", "Show a dialog")
            .with_parameters(json!({
                "type": "object",
                "properties": {
                    "title": {"type": "string"},
                    "message": {"type": "string"}
                },
                "required": ["title"]
            }));

        assert!(tool.is_frontend());
        let params = tool.parameters.unwrap();
        assert_eq!(params["type"], "object");
        assert!(params["properties"]["title"].is_object());
    }

    #[test]
    fn test_ag_ui_tool_def_serialization_frontend() {
        let tool = AGUIToolDef::frontend("copyToClipboard", "Copy text");
        let json = serde_json::to_string(&tool).unwrap();

        // Frontend tools should include execute field
        assert!(json.contains(r#""execute":"frontend""#));
        assert!(json.contains(r#""name":"copyToClipboard""#));
    }

    #[test]
    fn test_ag_ui_tool_def_serialization_backend_omits_execute() {
        let tool = AGUIToolDef::backend("search", "Search");
        let json = serde_json::to_string(&tool).unwrap();

        // Backend tools should omit execute field (it's the default)
        assert!(!json.contains("execute"));
    }

    #[test]
    fn test_ag_ui_tool_def_deserialization_with_execute() {
        let json = r#"{"name":"test","description":"Test","execute":"frontend"}"#;
        let tool: AGUIToolDef = serde_json::from_str(json).unwrap();
        assert_eq!(tool.execute, ToolExecutionLocation::Frontend);
        assert!(tool.is_frontend());
    }

    #[test]
    fn test_ag_ui_tool_def_deserialization_without_execute() {
        let json = r#"{"name":"test","description":"Test"}"#;
        let tool: AGUIToolDef = serde_json::from_str(json).unwrap();
        // Should default to backend
        assert_eq!(tool.execute, ToolExecutionLocation::Backend);
        assert!(!tool.is_frontend());
    }

    #[test]
    fn test_run_agent_request_frontend_tools() {
        let request = RunAgentRequest::new("t1".to_string(), "r1".to_string())
            .with_tool(AGUIToolDef::backend("search", "Search"))
            .with_tool(AGUIToolDef::frontend("copyToClipboard", "Copy"))
            .with_tool(AGUIToolDef::backend("read_file", "Read file"))
            .with_tool(AGUIToolDef::frontend("showNotification", "Show notification"));

        let frontend = request.frontend_tools();
        let backend = request.backend_tools();

        assert_eq!(frontend.len(), 2);
        assert_eq!(backend.len(), 2);

        assert_eq!(frontend[0].name, "copyToClipboard");
        assert_eq!(frontend[1].name, "showNotification");
        assert_eq!(backend[0].name, "search");
        assert_eq!(backend[1].name, "read_file");
    }

    #[test]
    fn test_run_agent_request_is_frontend_tool() {
        let request = RunAgentRequest::new("t1".to_string(), "r1".to_string())
            .with_tool(AGUIToolDef::backend("search", "Search"))
            .with_tool(AGUIToolDef::frontend("copyToClipboard", "Copy"));

        assert!(!request.is_frontend_tool("search"));
        assert!(request.is_frontend_tool("copyToClipboard"));
        assert!(!request.is_frontend_tool("nonexistent"));
    }

    #[test]
    fn test_run_agent_request_deserialization_with_frontend_tools() {
        let json = r#"{
            "threadId": "t1",
            "runId": "r1",
            "messages": [],
            "tools": [
                {"name": "search", "description": "Search the web"},
                {"name": "copyToClipboard", "description": "Copy text", "execute": "frontend"},
                {"name": "showDialog", "description": "Show dialog", "execute": "frontend", "parameters": {"type": "object"}}
            ]
        }"#;

        let request: RunAgentRequest = serde_json::from_str(json).unwrap();

        assert_eq!(request.tools.len(), 3);
        assert!(!request.is_frontend_tool("search"));
        assert!(request.is_frontend_tool("copyToClipboard"));
        assert!(request.is_frontend_tool("showDialog"));

        let frontend = request.frontend_tools();
        assert_eq!(frontend.len(), 2);
    }

    #[test]
    fn test_run_agent_request_empty_tools() {
        let request = RunAgentRequest::new("t1".to_string(), "r1".to_string());

        assert!(request.frontend_tools().is_empty());
        assert!(request.backend_tools().is_empty());
        assert!(!request.is_frontend_tool("anything"));
    }

    #[test]
    fn test_run_agent_request_all_frontend_tools() {
        let request = RunAgentRequest::new("t1".to_string(), "r1".to_string())
            .with_tool(AGUIToolDef::frontend("copy", "Copy"))
            .with_tool(AGUIToolDef::frontend("paste", "Paste"));

        assert!(request.backend_tools().is_empty());
        assert_eq!(request.frontend_tools().len(), 2);
    }

    #[test]
    fn test_run_agent_request_all_backend_tools() {
        let request = RunAgentRequest::new("t1".to_string(), "r1".to_string())
            .with_tool(AGUIToolDef::backend("search", "Search"))
            .with_tool(AGUIToolDef::backend("read", "Read"));

        assert!(request.frontend_tools().is_empty());
        assert_eq!(request.backend_tools().len(), 2);
    }
}
