//! AI SDK v6 compatible UI Message Stream types.
//!
//! This module provides types for the AI SDK v6 UI Message Stream protocol,
//! enabling direct integration with Vercel AI SDK frontend components.
//!
//! # Protocol Overview
//!
//! Events follow a start/delta/end pattern for streaming content.
//! The actual transport (SSE, WebSocket, etc.) is handled at a higher layer.
//!
//! # Example Event Sequence
//!
//! ```text
//! MessageStart { message_id: "msg_1" }
//! TextStart { id: "txt_1" }
//! TextDelta { id: "txt_1", delta: "Hello" }
//! TextEnd { id: "txt_1" }
//! Finish
//! ```

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Stream event types compatible with AI SDK v6.
///
/// These events map directly to the AI SDK UI Message Stream protocol.
/// See: https://ai-sdk.dev/docs/ai-sdk-ui/stream-protocol
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum UIStreamEvent {
    // ========================================================================
    // Message Lifecycle
    // ========================================================================
    /// Indicates the beginning of a new message with metadata.
    ///
    /// AI SDK v6 expects this as `{"type":"start","messageId":"..."}`.
    #[serde(rename = "start")]
    MessageStart {
        /// Unique identifier for this message.
        #[serde(rename = "messageId", skip_serializing_if = "Option::is_none")]
        message_id: Option<String>,
        /// Optional message metadata.
        #[serde(rename = "messageMetadata", skip_serializing_if = "Option::is_none")]
        message_metadata: Option<Value>,
    },

    // ========================================================================
    // Text Streaming (start/delta/end pattern)
    // ========================================================================
    /// Indicates the beginning of a text block.
    TextStart {
        /// Unique identifier for this text block.
        id: String,
    },

    /// Contains incremental text content for the text block.
    TextDelta {
        /// Identifier matching the text-start event.
        id: String,
        /// Incremental text content.
        delta: String,
    },

    /// Indicates the end of a text block.
    TextEnd {
        /// Identifier matching the text-start event.
        id: String,
    },

    // ========================================================================
    // Reasoning Streaming (for models like DeepSeek, o1)
    // ========================================================================
    /// Indicates the beginning of a reasoning block.
    ReasoningStart {
        /// Unique identifier for this reasoning block.
        id: String,
    },

    /// Contains incremental reasoning content.
    ReasoningDelta {
        /// Identifier matching the reasoning-start event.
        id: String,
        /// Incremental reasoning content.
        delta: String,
    },

    /// Indicates the end of a reasoning block.
    ReasoningEnd {
        /// Identifier matching the reasoning-start event.
        id: String,
    },

    // ========================================================================
    // Tool Input Streaming
    // ========================================================================
    /// Indicates the beginning of tool input streaming.
    ToolInputStart {
        /// Unique identifier for this tool call.
        #[serde(rename = "toolCallId")]
        tool_call_id: String,
        /// Name of the tool being called.
        #[serde(rename = "toolName")]
        tool_name: String,
    },

    /// Contains incremental chunks of tool input as it's being generated.
    ToolInputDelta {
        /// Identifier matching the tool-input-start event.
        #[serde(rename = "toolCallId")]
        tool_call_id: String,
        /// Incremental tool input text.
        #[serde(rename = "inputTextDelta")]
        input_text_delta: String,
    },

    /// Indicates that tool input is complete and ready for execution.
    ToolInputAvailable {
        /// Identifier matching the tool-input-start event.
        #[serde(rename = "toolCallId")]
        tool_call_id: String,
        /// Name of the tool being called.
        #[serde(rename = "toolName")]
        tool_name: String,
        /// Complete tool input as JSON.
        input: Value,
    },

    // ========================================================================
    // Tool Output
    // ========================================================================
    /// Contains the result of tool execution.
    ToolOutputAvailable {
        /// Identifier matching the tool-input-start event.
        #[serde(rename = "toolCallId")]
        tool_call_id: String,
        /// Tool execution result as JSON.
        output: Value,
    },

    // ========================================================================
    // Step Boundaries (for multi-step agents)
    // ========================================================================
    /// Marks the beginning of a step.
    StartStep,

    /// Marks the completion of an LLM API call step.
    FinishStep,

    // ========================================================================
    // Source References (for RAG)
    // ========================================================================
    /// References an external URL.
    SourceUrl {
        /// Unique identifier for this source.
        #[serde(rename = "sourceId")]
        source_id: String,
        /// The URL being referenced.
        url: String,
        /// Optional title for the source.
        #[serde(skip_serializing_if = "Option::is_none")]
        title: Option<String>,
    },

    /// References a document or file.
    SourceDocument {
        /// Unique identifier for this source.
        #[serde(rename = "sourceId")]
        source_id: String,
        /// IANA media type of the document.
        #[serde(rename = "mediaType")]
        media_type: String,
        /// Title of the document.
        title: String,
        /// Optional filename.
        #[serde(skip_serializing_if = "Option::is_none")]
        filename: Option<String>,
    },

    /// Contains a file reference.
    ///
    /// AI SDK v6 strict schema only includes `url`, `mediaType`, and optional `providerMetadata`.
    File {
        /// URL to the file.
        url: String,
        /// IANA media type.
        #[serde(rename = "mediaType")]
        media_type: String,
        /// Optional provider metadata.
        #[serde(rename = "providerMetadata", skip_serializing_if = "Option::is_none")]
        provider_metadata: Option<Value>,
    },

    // ========================================================================
    // Stream Lifecycle
    // ========================================================================
    /// Indicates message completion.
    Finish {
        /// Optional reason for finishing (stop, length, content-filter, tool-calls, error, other).
        #[serde(rename = "finishReason", skip_serializing_if = "Option::is_none")]
        finish_reason: Option<String>,
        /// Optional message metadata.
        #[serde(rename = "messageMetadata", skip_serializing_if = "Option::is_none")]
        message_metadata: Option<Value>,
    },

    /// Signals stream abortion with a reason.
    Abort {
        /// Optional reason for the abort.
        #[serde(skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
    },

    /// Appends error messages to stream.
    Error {
        /// Error text.
        #[serde(rename = "errorText")]
        error_text: String,
    },

    // ========================================================================
    // Custom Data (extensible)
    // ========================================================================
    /// Custom data event with a type prefix pattern (data-*).
    #[serde(untagged)]
    Data {
        /// Custom type name (should start with "data-").
        #[serde(rename = "type")]
        data_type: String,
        /// Custom data payload.
        data: Value,
    },
}

impl UIStreamEvent {
    // ========================================================================
    // Factory Methods
    // ========================================================================

    /// Create a start event (message start).
    pub fn message_start(message_id: impl Into<String>) -> Self {
        Self::MessageStart {
            message_id: Some(message_id.into()),
            message_metadata: None,
        }
    }

    /// Create a start event without message ID.
    pub fn start() -> Self {
        Self::MessageStart {
            message_id: None,
            message_metadata: None,
        }
    }

    /// Create a text-start event.
    pub fn text_start(id: impl Into<String>) -> Self {
        Self::TextStart { id: id.into() }
    }

    /// Create a text-delta event.
    pub fn text_delta(id: impl Into<String>, delta: impl Into<String>) -> Self {
        Self::TextDelta {
            id: id.into(),
            delta: delta.into(),
        }
    }

    /// Create a text-end event.
    pub fn text_end(id: impl Into<String>) -> Self {
        Self::TextEnd { id: id.into() }
    }

    /// Create a reasoning-start event.
    pub fn reasoning_start(id: impl Into<String>) -> Self {
        Self::ReasoningStart { id: id.into() }
    }

    /// Create a reasoning-delta event.
    pub fn reasoning_delta(id: impl Into<String>, delta: impl Into<String>) -> Self {
        Self::ReasoningDelta {
            id: id.into(),
            delta: delta.into(),
        }
    }

    /// Create a reasoning-end event.
    pub fn reasoning_end(id: impl Into<String>) -> Self {
        Self::ReasoningEnd { id: id.into() }
    }

    /// Create a tool-input-start event.
    pub fn tool_input_start(tool_call_id: impl Into<String>, tool_name: impl Into<String>) -> Self {
        Self::ToolInputStart {
            tool_call_id: tool_call_id.into(),
            tool_name: tool_name.into(),
        }
    }

    /// Create a tool-input-delta event.
    pub fn tool_input_delta(tool_call_id: impl Into<String>, delta: impl Into<String>) -> Self {
        Self::ToolInputDelta {
            tool_call_id: tool_call_id.into(),
            input_text_delta: delta.into(),
        }
    }

    /// Create a tool-input-available event.
    pub fn tool_input_available(
        tool_call_id: impl Into<String>,
        tool_name: impl Into<String>,
        input: Value,
    ) -> Self {
        Self::ToolInputAvailable {
            tool_call_id: tool_call_id.into(),
            tool_name: tool_name.into(),
            input,
        }
    }

    /// Create a tool-output-available event.
    pub fn tool_output_available(tool_call_id: impl Into<String>, output: Value) -> Self {
        Self::ToolOutputAvailable {
            tool_call_id: tool_call_id.into(),
            output,
        }
    }

    /// Create a start-step event.
    pub fn start_step() -> Self {
        Self::StartStep
    }

    /// Create a finish-step event.
    pub fn finish_step() -> Self {
        Self::FinishStep
    }

    /// Create a source-url event.
    pub fn source_url(
        source_id: impl Into<String>,
        url: impl Into<String>,
        title: Option<String>,
    ) -> Self {
        Self::SourceUrl {
            source_id: source_id.into(),
            url: url.into(),
            title,
        }
    }

    /// Create a source-document event.
    pub fn source_document(
        source_id: impl Into<String>,
        media_type: impl Into<String>,
        title: impl Into<String>,
        filename: Option<String>,
    ) -> Self {
        Self::SourceDocument {
            source_id: source_id.into(),
            media_type: media_type.into(),
            title: title.into(),
            filename,
        }
    }

    /// Create a file event.
    pub fn file(url: impl Into<String>, media_type: impl Into<String>) -> Self {
        Self::File {
            url: url.into(),
            media_type: media_type.into(),
            provider_metadata: None,
        }
    }

    /// Create a finish event.
    pub fn finish() -> Self {
        Self::Finish {
            finish_reason: None,
            message_metadata: None,
        }
    }

    /// Create a finish event with a reason.
    pub fn finish_with_reason(reason: impl Into<String>) -> Self {
        Self::Finish {
            finish_reason: Some(reason.into()),
            message_metadata: None,
        }
    }

    /// Create an abort event.
    pub fn abort(reason: impl Into<String>) -> Self {
        Self::Abort {
            reason: Some(reason.into()),
        }
    }

    /// Create an error event.
    pub fn error(error_text: impl Into<String>) -> Self {
        Self::Error {
            error_text: error_text.into(),
        }
    }

    /// Create a custom data event.
    pub fn data(name: impl Into<String>, data: Value) -> Self {
        let data_type = format!("data-{}", name.into());
        Self::Data { data_type, data }
    }
}

// ============================================================================
// UIMessage Types (for persistence and state)
// ============================================================================

/// Streaming state for UI parts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum StreamState {
    /// Content is still streaming.
    Streaming,
    /// Content streaming is complete.
    Done,
}

/// Tool execution state in UI.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ToolState {
    /// Tool input is being streamed.
    InputStreaming,
    /// Tool input is complete, ready for execution.
    InputAvailable,
    /// Tool execution completed with output.
    OutputAvailable,
    /// Tool execution resulted in error.
    OutputError,
}

/// A part of a UI message.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum UIMessagePart {
    /// Text content part.
    Text {
        /// The text content.
        text: String,
        /// Optional streaming state.
        #[serde(skip_serializing_if = "Option::is_none")]
        state: Option<StreamState>,
    },

    /// Reasoning content part.
    Reasoning {
        /// The reasoning text.
        text: String,
        /// Optional streaming state.
        #[serde(skip_serializing_if = "Option::is_none")]
        state: Option<StreamState>,
        /// Optional provider metadata.
        #[serde(rename = "providerMetadata", skip_serializing_if = "Option::is_none")]
        provider_metadata: Option<Value>,
    },

    /// Tool invocation part.
    Tool {
        /// Tool call identifier.
        #[serde(rename = "toolCallId")]
        tool_call_id: String,
        /// Tool name.
        #[serde(rename = "toolName")]
        tool_name: String,
        /// Tool execution state.
        state: ToolState,
        /// Tool input (available after input-available).
        #[serde(skip_serializing_if = "Option::is_none")]
        input: Option<Value>,
        /// Tool output (available after output-available).
        #[serde(skip_serializing_if = "Option::is_none")]
        output: Option<Value>,
        /// Error text (if output-error state).
        #[serde(rename = "errorText", skip_serializing_if = "Option::is_none")]
        error_text: Option<String>,
    },

    /// URL source reference.
    SourceUrl {
        /// Source identifier.
        #[serde(rename = "sourceId")]
        source_id: String,
        /// The URL.
        url: String,
        /// Optional title.
        #[serde(skip_serializing_if = "Option::is_none")]
        title: Option<String>,
        /// Optional provider metadata.
        #[serde(rename = "providerMetadata", skip_serializing_if = "Option::is_none")]
        provider_metadata: Option<Value>,
    },

    /// Document source reference.
    SourceDocument {
        /// Source identifier.
        #[serde(rename = "sourceId")]
        source_id: String,
        /// IANA media type.
        #[serde(rename = "mediaType")]
        media_type: String,
        /// Document title.
        title: String,
        /// Optional filename.
        #[serde(skip_serializing_if = "Option::is_none")]
        filename: Option<String>,
        /// Optional provider metadata.
        #[serde(rename = "providerMetadata", skip_serializing_if = "Option::is_none")]
        provider_metadata: Option<Value>,
    },

    /// File attachment.
    File {
        /// File URL.
        url: String,
        /// IANA media type.
        #[serde(rename = "mediaType")]
        media_type: String,
    },

    /// Step start marker.
    StepStart,

    /// Custom data part.
    #[serde(untagged)]
    Data {
        /// Custom type.
        #[serde(rename = "type")]
        data_type: String,
        /// Data payload.
        data: Value,
    },
}

/// Role in the conversation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum UIRole {
    /// System message.
    System,
    /// User message.
    User,
    /// Assistant message.
    Assistant,
}

/// A UI message with rich parts.
///
/// This is the source of truth for application state, representing the complete
/// message including metadata, data parts, and all contextual information.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct UIMessage {
    /// Unique identifier for this message.
    pub id: String,
    /// Role of the message sender.
    pub role: UIRole,
    /// Optional metadata.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Value>,
    /// Message parts.
    pub parts: Vec<UIMessagePart>,
}

impl UIMessage {
    /// Create a new UI message.
    pub fn new(id: impl Into<String>, role: UIRole) -> Self {
        Self {
            id: id.into(),
            role,
            metadata: None,
            parts: Vec::new(),
        }
    }

    /// Create a user message.
    pub fn user(id: impl Into<String>, text: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            role: UIRole::User,
            metadata: None,
            parts: vec![UIMessagePart::Text {
                text: text.into(),
                state: Some(StreamState::Done),
            }],
        }
    }

    /// Create an assistant message.
    pub fn assistant(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            role: UIRole::Assistant,
            metadata: None,
            parts: Vec::new(),
        }
    }

    /// Add metadata.
    #[must_use]
    pub fn with_metadata(mut self, metadata: Value) -> Self {
        self.metadata = Some(metadata);
        self
    }

    /// Add a part.
    #[must_use]
    pub fn with_part(mut self, part: UIMessagePart) -> Self {
        self.parts.push(part);
        self
    }

    /// Add multiple parts.
    #[must_use]
    pub fn with_parts(mut self, parts: impl IntoIterator<Item = UIMessagePart>) -> Self {
        self.parts.extend(parts);
        self
    }

    /// Add a text part.
    #[must_use]
    pub fn with_text(self, text: impl Into<String>) -> Self {
        self.with_part(UIMessagePart::Text {
            text: text.into(),
            state: Some(StreamState::Done),
        })
    }

    /// Get all text content concatenated.
    pub fn text_content(&self) -> String {
        self.parts
            .iter()
            .filter_map(|p| match p {
                UIMessagePart::Text { text, .. } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("")
    }
}

// ============================================================================
// AI SDK Adapter
// ============================================================================

/// Adapter for AI SDK v6 UI Message Stream protocol.
///
/// Converts `AgentEvent` to `UIStreamEvent` for Vercel AI SDK integration.
///
/// # Example
///
/// ```rust
/// use carve_agent::{AgentEvent, AiSdkAdapter};
///
/// let adapter = AiSdkAdapter::new("run_123".to_string());
///
/// // Convert event to SSE format
/// let event = AgentEvent::TextDelta { delta: "Hello".to_string() };
/// let sse_lines = adapter.to_sse(&event);
/// assert!(sse_lines[0].starts_with("data: "));
/// ```
#[derive(Debug, Clone)]
pub struct AiSdkAdapter {
    /// Text block identifier for streaming.
    text_id: String,
    /// Message identifier.
    message_id: String,
    /// Run identifier.
    run_id: String,
}

impl AiSdkAdapter {
    /// Create a new AI SDK adapter.
    pub fn new(run_id: String) -> Self {
        let message_id = format!("msg_{}", &run_id[..8.min(run_id.len())]);
        Self {
            text_id: "txt_0".to_string(),
            message_id,
            run_id,
        }
    }

    /// Get the message ID.
    pub fn message_id(&self) -> &str {
        &self.message_id
    }

    /// Get the run ID.
    pub fn run_id(&self) -> &str {
        &self.run_id
    }

    /// Get the text ID.
    pub fn text_id(&self) -> &str {
        &self.text_id
    }

    /// Convert an AgentEvent to UIStreamEvent(s).
    pub fn convert(&self, event: &crate::stream::AgentEvent) -> Vec<UIStreamEvent> {
        crate::stream::agent_event_to_ui(event, &self.text_id)
    }

    /// Convert an AgentEvent to JSON strings.
    pub fn to_json(&self, event: &crate::stream::AgentEvent) -> Vec<String> {
        self.convert(event)
            .into_iter()
            .filter_map(|e| serde_json::to_string(&e).ok())
            .collect()
    }

    /// Convert an AgentEvent to SSE (Server-Sent Events) format.
    ///
    /// Returns lines in the format: `data: {json}\n\n`
    pub fn to_sse(&self, event: &crate::stream::AgentEvent) -> Vec<String> {
        self.to_json(event)
            .into_iter()
            .map(|json| format!("data: {}\n\n", json))
            .collect()
    }

    /// Convert an AgentEvent to newline-delimited JSON (NDJSON) format.
    ///
    /// Returns lines in the format: `{json}\n`
    pub fn to_ndjson(&self, event: &crate::stream::AgentEvent) -> Vec<String> {
        self.to_json(event)
            .into_iter()
            .map(|json| format!("{}\n", json))
            .collect()
    }

    /// Generate message start event.
    pub fn message_start(&self) -> UIStreamEvent {
        UIStreamEvent::message_start(&self.message_id)
    }

    /// Generate text start event.
    pub fn text_start(&self) -> UIStreamEvent {
        UIStreamEvent::text_start(&self.text_id)
    }

    /// Generate text end event.
    pub fn text_end(&self) -> UIStreamEvent {
        UIStreamEvent::text_end(&self.text_id)
    }

    /// Generate finish event.
    pub fn finish(&self) -> UIStreamEvent {
        UIStreamEvent::finish()
    }
}

impl Default for AiSdkAdapter {
    fn default() -> Self {
        Self::new("default".to_string())
    }
}

// ============================================================================
// AI SDK Encoder (stateful text lifecycle tracking)
// ============================================================================

/// Stateful encoder for AI SDK v6 UI Message Stream protocol.
///
/// Tracks text block lifecycle (open/close) across tool calls, ensuring
/// `text-start` and `text-end` are always properly paired. This mirrors the
/// pattern used by [`AGUIContext`](crate::ag_ui::AGUIContext) for AG-UI.
///
/// # Text lifecycle rules
///
/// - `TextDelta` with text closed → prepend `text-start`, open text
/// - `ToolCallStart` with text open → prepend `text-end`, close text
/// - `RunFinish` with text open → prepend `text-end` before `finish`
/// - `Error`/`Aborted` → terminal, no `text-end` needed
#[derive(Debug)]
pub struct AiSdkEncoder {
    message_id: String,
    run_id: String,
    text_open: bool,
    text_counter: u32,
    finished: bool,
}

impl AiSdkEncoder {
    /// Create a new encoder for the given run.
    pub fn new(run_id: String) -> Self {
        let message_id = format!("msg_{}", &run_id[..8.min(run_id.len())]);
        Self {
            message_id,
            run_id,
            text_open: false,
            text_counter: 0,
            finished: false,
        }
    }

    /// Current text block ID (e.g. `txt_0`, `txt_1`, ...).
    fn text_id(&self) -> String {
        format!("txt_{}", self.text_counter)
    }

    /// Emit `text-start` and mark text as open. Returns the new text ID.
    fn open_text(&mut self) -> UIStreamEvent {
        self.text_open = true;
        UIStreamEvent::text_start(self.text_id())
    }

    /// Emit `text-end` for the current text block and mark text as closed.
    /// Increments the counter so the next text block gets a fresh ID.
    fn close_text(&mut self) -> UIStreamEvent {
        let event = UIStreamEvent::text_end(self.text_id());
        self.text_open = false;
        self.text_counter += 1;
        event
    }

    /// Get the message ID.
    pub fn message_id(&self) -> &str {
        &self.message_id
    }

    /// Get the run ID.
    pub fn run_id(&self) -> &str {
        &self.run_id
    }

    /// Emit the stream prologue: `message-start`.
    ///
    /// Unlike the old adapter, this does NOT emit `text-start` here —
    /// text blocks are opened lazily when the first `TextDelta` arrives.
    pub fn prologue(&self) -> Vec<UIStreamEvent> {
        vec![UIStreamEvent::message_start(&self.message_id)]
    }

    /// Convert an `AgentEvent` to UI stream events with proper text lifecycle.
    pub fn on_agent_event(&mut self, ev: &crate::stream::AgentEvent) -> Vec<UIStreamEvent> {
        use crate::stream::AgentEvent;

        if self.finished {
            return Vec::new();
        }

        match ev {
            AgentEvent::TextDelta { delta } => {
                let mut events = Vec::new();
                if !self.text_open {
                    events.push(self.open_text());
                }
                events.push(UIStreamEvent::text_delta(self.text_id(), delta));
                events
            }

            AgentEvent::ToolCallStart { id, name } => {
                let mut events = Vec::new();
                if self.text_open {
                    events.push(self.close_text());
                }
                events.push(UIStreamEvent::tool_input_start(id, name));
                events
            }
            AgentEvent::ToolCallDelta { id, args_delta } => {
                vec![UIStreamEvent::tool_input_delta(id, args_delta)]
            }
            AgentEvent::ToolCallReady {
                id,
                name,
                arguments,
            } => {
                vec![UIStreamEvent::tool_input_available(
                    id,
                    name,
                    arguments.clone(),
                )]
            }
            AgentEvent::ToolCallDone { id, result, .. } => {
                vec![UIStreamEvent::tool_output_available(id, result.to_json())]
            }

            AgentEvent::RunFinish { stop_reason, .. } => {
                self.finished = true;
                let mut events = Vec::new();
                if self.text_open {
                    events.push(self.close_text());
                }
                let finish_reason = Self::map_stop_reason(stop_reason.as_ref());
                events.push(UIStreamEvent::finish_with_reason(finish_reason));
                events
            }

            AgentEvent::Error { message } => {
                self.finished = true;
                // Terminal — no text-end needed (matches AG-UI behavior).
                self.text_open = false;
                vec![UIStreamEvent::error(message)]
            }
            AgentEvent::Aborted { reason } => {
                self.finished = true;
                self.text_open = false;
                vec![UIStreamEvent::abort(reason)]
            }

            AgentEvent::StepStart => vec![UIStreamEvent::start_step()],
            AgentEvent::StepEnd => vec![UIStreamEvent::finish_step()],
            AgentEvent::RunStart { .. } | AgentEvent::InferenceComplete { .. } => vec![],

            AgentEvent::StateSnapshot { snapshot } => {
                vec![UIStreamEvent::data("state-snapshot", snapshot.clone())]
            }
            AgentEvent::StateDelta { delta } => {
                vec![UIStreamEvent::data(
                    "state-delta",
                    serde_json::Value::Array(delta.clone()),
                )]
            }
            AgentEvent::MessagesSnapshot { messages } => {
                vec![UIStreamEvent::data(
                    "messages-snapshot",
                    serde_json::Value::Array(messages.clone()),
                )]
            }
            AgentEvent::ActivitySnapshot {
                message_id,
                activity_type,
                content,
                replace,
            } => {
                let payload = serde_json::json!({
                    "messageId": message_id,
                    "activityType": activity_type,
                    "content": content,
                    "replace": replace,
                });
                vec![UIStreamEvent::data("activity-snapshot", payload)]
            }
            AgentEvent::ActivityDelta {
                message_id,
                activity_type,
                patch,
            } => {
                let payload = serde_json::json!({
                    "messageId": message_id,
                    "activityType": activity_type,
                    "patch": patch,
                });
                vec![UIStreamEvent::data("activity-delta", payload)]
            }
            AgentEvent::Pending { interaction } => {
                vec![UIStreamEvent::data(
                    "interaction",
                    serde_json::to_value(interaction).unwrap_or_default(),
                )]
            }
        }
    }

    fn map_stop_reason(reason: Option<&crate::stop::StopReason>) -> &'static str {
        use crate::stop::StopReason;
        match reason {
            Some(StopReason::NaturalEnd)
            | Some(StopReason::ContentMatched(_))
            | Some(StopReason::PluginRequested) => "stop",
            Some(StopReason::MaxRoundsReached)
            | Some(StopReason::TimeoutReached)
            | Some(StopReason::TokenBudgetExceeded) => "length",
            Some(StopReason::ToolCalled(_)) => "tool-calls",
            Some(StopReason::Cancelled) => "other",
            Some(StopReason::ConsecutiveErrorsExceeded) | Some(StopReason::LoopDetected) => {
                "error"
            }
            Some(StopReason::Custom(_)) => "other",
            None => "stop",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // ========================================================================
    // UIStreamEvent Serialization Tests
    // ========================================================================

    #[test]
    fn test_message_start_serialization() {
        let event = UIStreamEvent::message_start("msg_123");
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains(r#""type":"start""#));
        assert!(json.contains(r#""messageId":"msg_123""#));
    }

    #[test]
    fn test_text_start_serialization() {
        let event = UIStreamEvent::text_start("txt_1");
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains(r#""type":"text-start""#));
        assert!(json.contains(r#""id":"txt_1""#));
    }

    #[test]
    fn test_text_delta_serialization() {
        let event = UIStreamEvent::text_delta("txt_1", "Hello ");
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains(r#""type":"text-delta""#));
        assert!(json.contains(r#""id":"txt_1""#));
        assert!(json.contains(r#""delta":"Hello ""#));
    }

    #[test]
    fn test_text_end_serialization() {
        let event = UIStreamEvent::text_end("txt_1");
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains(r#""type":"text-end""#));
        assert!(json.contains(r#""id":"txt_1""#));
    }

    #[test]
    fn test_reasoning_events_serialization() {
        let start = UIStreamEvent::reasoning_start("r_1");
        let delta = UIStreamEvent::reasoning_delta("r_1", "thinking...");
        let end = UIStreamEvent::reasoning_end("r_1");

        assert!(serde_json::to_string(&start)
            .unwrap()
            .contains(r#""type":"reasoning-start""#));
        assert!(serde_json::to_string(&delta)
            .unwrap()
            .contains(r#""type":"reasoning-delta""#));
        assert!(serde_json::to_string(&end)
            .unwrap()
            .contains(r#""type":"reasoning-end""#));
    }

    #[test]
    fn test_tool_input_start_serialization() {
        let event = UIStreamEvent::tool_input_start("call_1", "search");
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains(r#""type":"tool-input-start""#));
        assert!(json.contains(r#""toolCallId":"call_1""#));
        assert!(json.contains(r#""toolName":"search""#));
    }

    #[test]
    fn test_tool_input_delta_serialization() {
        let event = UIStreamEvent::tool_input_delta("call_1", r#"{"query":"#);
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains(r#""type":"tool-input-delta""#));
        assert!(json.contains(r#""toolCallId":"call_1""#));
        assert!(json.contains(r#""inputTextDelta""#));
    }

    #[test]
    fn test_tool_input_available_serialization() {
        let event =
            UIStreamEvent::tool_input_available("call_1", "search", json!({"query": "rust"}));
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains(r#""type":"tool-input-available""#));
        assert!(json.contains(r#""toolCallId":"call_1""#));
        assert!(json.contains(r#""toolName":"search""#));
        assert!(json.contains(r#""input""#));
    }

    #[test]
    fn test_tool_output_available_serialization() {
        let event =
            UIStreamEvent::tool_output_available("call_1", json!({"result": "found 3 items"}));
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains(r#""type":"tool-output-available""#));
        assert!(json.contains(r#""toolCallId":"call_1""#));
        assert!(json.contains(r#""output""#));
    }

    #[test]
    fn test_step_events_serialization() {
        let start = UIStreamEvent::start_step();
        let finish = UIStreamEvent::finish_step();

        assert!(serde_json::to_string(&start)
            .unwrap()
            .contains(r#""type":"start-step""#));
        assert!(serde_json::to_string(&finish)
            .unwrap()
            .contains(r#""type":"finish-step""#));
    }

    #[test]
    fn test_source_url_serialization() {
        let event =
            UIStreamEvent::source_url("src_1", "https://example.com", Some("Example".to_string()));
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains(r#""type":"source-url""#));
        assert!(json.contains(r#""sourceId":"src_1""#));
        assert!(json.contains(r#""url":"https://example.com""#));
        assert!(json.contains(r#""title":"Example""#));
    }

    #[test]
    fn test_source_url_without_title() {
        let event = UIStreamEvent::source_url("src_1", "https://example.com", None);
        let json = serde_json::to_string(&event).unwrap();
        assert!(!json.contains("title"));
    }

    #[test]
    fn test_source_document_serialization() {
        let event = UIStreamEvent::source_document(
            "src_1",
            "application/pdf",
            "Document Title",
            Some("doc.pdf".to_string()),
        );
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains(r#""type":"source-document""#));
        assert!(json.contains(r#""mediaType":"application/pdf""#));
        assert!(json.contains(r#""title":"Document Title""#));
        assert!(json.contains(r#""filename":"doc.pdf""#));
    }

    #[test]
    fn test_file_serialization() {
        let event = UIStreamEvent::file("https://example.com/file.png", "image/png");
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains(r#""type":"file""#));
        assert!(json.contains(r#""url":"https://example.com/file.png""#));
        assert!(json.contains(r#""mediaType":"image/png""#));
    }

    #[test]
    fn test_finish_serialization() {
        let event = UIStreamEvent::finish();
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains(r#""type":"finish""#));
    }

    #[test]
    fn test_abort_serialization() {
        let event = UIStreamEvent::abort("User cancelled");
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains(r#""type":"abort""#));
        assert!(json.contains(r#""reason":"User cancelled""#));
    }

    #[test]
    fn test_error_serialization() {
        let event = UIStreamEvent::error("Something went wrong");
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains(r#""type":"error""#));
        assert!(json.contains(r#""errorText":"Something went wrong""#));
    }

    // ========================================================================
    // Deserialization Tests
    // ========================================================================

    #[test]
    fn test_message_start_deserialization() {
        let json = r#"{"type":"start","messageId":"msg_123"}"#;
        let event: UIStreamEvent = serde_json::from_str(json).unwrap();
        assert_eq!(event, UIStreamEvent::message_start("msg_123"));
    }

    #[test]
    fn test_text_delta_deserialization() {
        let json = r#"{"type":"text-delta","id":"txt_1","delta":"Hello"}"#;
        let event: UIStreamEvent = serde_json::from_str(json).unwrap();
        assert_eq!(event, UIStreamEvent::text_delta("txt_1", "Hello"));
    }

    #[test]
    fn test_tool_input_available_deserialization() {
        let json = r#"{"type":"tool-input-available","toolCallId":"call_1","toolName":"search","input":{"query":"rust"}}"#;
        let event: UIStreamEvent = serde_json::from_str(json).unwrap();
        if let UIStreamEvent::ToolInputAvailable {
            tool_call_id,
            tool_name,
            input,
        } = event
        {
            assert_eq!(tool_call_id, "call_1");
            assert_eq!(tool_name, "search");
            assert_eq!(input["query"], "rust");
        } else {
            panic!("Expected ToolInputAvailable");
        }
    }

    // ========================================================================
    // UIMessage Tests
    // ========================================================================

    #[test]
    fn test_ui_message_user() {
        let msg = UIMessage::user("msg_1", "Hello");
        assert_eq!(msg.id, "msg_1");
        assert_eq!(msg.role, UIRole::User);
        assert_eq!(msg.text_content(), "Hello");
    }

    #[test]
    fn test_ui_message_assistant() {
        let msg = UIMessage::assistant("msg_1").with_text("Hi there!");
        assert_eq!(msg.role, UIRole::Assistant);
        assert_eq!(msg.text_content(), "Hi there!");
    }

    #[test]
    fn test_ui_message_with_metadata() {
        let msg = UIMessage::user("msg_1", "Hello").with_metadata(json!({"timestamp": 12345}));
        assert!(msg.metadata.is_some());
        assert_eq!(msg.metadata.as_ref().unwrap()["timestamp"], 12345);
    }

    #[test]
    fn test_ui_message_with_multiple_parts() {
        let msg = UIMessage::assistant("msg_1")
            .with_text("Let me search for that.")
            .with_part(UIMessagePart::Tool {
                tool_call_id: "call_1".to_string(),
                tool_name: "search".to_string(),
                state: ToolState::OutputAvailable,
                input: Some(json!({"query": "rust"})),
                output: Some(json!({"results": ["result1"]})),
                error_text: None,
            })
            .with_text(" Found it!");

        assert_eq!(msg.parts.len(), 3);
        assert_eq!(msg.text_content(), "Let me search for that. Found it!");
    }

    #[test]
    fn test_ui_message_serialization() {
        let msg = UIMessage::user("msg_1", "Hello");
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains(r#""id":"msg_1""#));
        assert!(json.contains(r#""role":"user""#));
        assert!(json.contains(r#""parts""#));
    }

    #[test]
    fn test_ui_message_deserialization() {
        let json = r#"{"id":"msg_1","role":"assistant","parts":[{"type":"text","text":"Hello","state":"done"}]}"#;
        let msg: UIMessage = serde_json::from_str(json).unwrap();
        assert_eq!(msg.id, "msg_1");
        assert_eq!(msg.role, UIRole::Assistant);
        assert_eq!(msg.text_content(), "Hello");
    }

    // ========================================================================
    // UIMessagePart Tests
    // ========================================================================

    #[test]
    fn test_text_part_serialization() {
        let part = UIMessagePart::Text {
            text: "Hello".to_string(),
            state: Some(StreamState::Done),
        };
        let json = serde_json::to_string(&part).unwrap();
        assert!(json.contains(r#""type":"text""#));
        assert!(json.contains(r#""text":"Hello""#));
        assert!(json.contains(r#""state":"done""#));
    }

    #[test]
    fn test_reasoning_part_serialization() {
        let part = UIMessagePart::Reasoning {
            text: "Thinking...".to_string(),
            state: Some(StreamState::Streaming),
            provider_metadata: None,
        };
        let json = serde_json::to_string(&part).unwrap();
        assert!(json.contains(r#""type":"reasoning""#));
        assert!(json.contains(r#""state":"streaming""#));
    }

    #[test]
    fn test_tool_part_serialization() {
        let part = UIMessagePart::Tool {
            tool_call_id: "call_1".to_string(),
            tool_name: "search".to_string(),
            state: ToolState::InputAvailable,
            input: Some(json!({"query": "rust"})),
            output: None,
            error_text: None,
        };
        let json = serde_json::to_string(&part).unwrap();
        assert!(json.contains(r#""type":"tool""#));
        assert!(json.contains(r#""state":"input-available""#));
        assert!(json.contains(r#""toolCallId":"call_1""#));
    }

    #[test]
    fn test_tool_part_with_error() {
        let part = UIMessagePart::Tool {
            tool_call_id: "call_1".to_string(),
            tool_name: "search".to_string(),
            state: ToolState::OutputError,
            input: Some(json!({"query": "rust"})),
            output: None,
            error_text: Some("Tool not found".to_string()),
        };
        let json = serde_json::to_string(&part).unwrap();
        assert!(json.contains(r#""state":"output-error""#));
        assert!(json.contains(r#""errorText":"Tool not found""#));
    }

    #[test]
    fn test_step_start_part_serialization() {
        let part = UIMessagePart::StepStart;
        let json = serde_json::to_string(&part).unwrap();
        assert!(json.contains(r#""type":"step-start""#));
    }

    // ========================================================================
    // Complete Flow Tests
    // ========================================================================

    #[test]
    fn test_complete_text_streaming_flow() {
        let events = [
            UIStreamEvent::message_start("msg_1"),
            UIStreamEvent::text_start("txt_1"),
            UIStreamEvent::text_delta("txt_1", "Hello "),
            UIStreamEvent::text_delta("txt_1", "World"),
            UIStreamEvent::text_delta("txt_1", "!"),
            UIStreamEvent::text_end("txt_1"),
            UIStreamEvent::finish(),
        ];

        // Verify event sequence
        assert_eq!(events.len(), 7);
        assert!(matches!(&events[0], UIStreamEvent::MessageStart { .. }));
        assert!(matches!(&events[1], UIStreamEvent::TextStart { .. }));
        assert!(matches!(&events[6], UIStreamEvent::Finish { .. }));
    }

    #[test]
    fn test_complete_tool_call_flow() {
        let events = vec![
            UIStreamEvent::message_start("msg_1"),
            UIStreamEvent::text_start("txt_1"),
            UIStreamEvent::text_delta("txt_1", "Let me search for that."),
            UIStreamEvent::text_end("txt_1"),
            UIStreamEvent::tool_input_start("call_1", "search"),
            UIStreamEvent::tool_input_delta("call_1", r#"{"query":"#),
            UIStreamEvent::tool_input_delta("call_1", r#""rust"}"#),
            UIStreamEvent::tool_input_available("call_1", "search", json!({"query": "rust"})),
            UIStreamEvent::tool_output_available("call_1", json!({"results": ["item1", "item2"]})),
            UIStreamEvent::finish(),
        ];

        // Verify tool events are present
        assert!(events
            .iter()
            .any(|e| matches!(e, UIStreamEvent::ToolInputStart { .. })));
        assert!(events
            .iter()
            .any(|e| matches!(e, UIStreamEvent::ToolInputDelta { .. })));
        assert!(events
            .iter()
            .any(|e| matches!(e, UIStreamEvent::ToolInputAvailable { .. })));
        assert!(events
            .iter()
            .any(|e| matches!(e, UIStreamEvent::ToolOutputAvailable { .. })));
    }

    #[test]
    fn test_multi_step_flow() {
        let events = vec![
            UIStreamEvent::message_start("msg_1"),
            // Step 1
            UIStreamEvent::start_step(),
            UIStreamEvent::text_start("txt_1"),
            UIStreamEvent::text_delta("txt_1", "Step 1 response"),
            UIStreamEvent::text_end("txt_1"),
            UIStreamEvent::finish_step(),
            // Step 2
            UIStreamEvent::start_step(),
            UIStreamEvent::tool_input_start("call_1", "search"),
            UIStreamEvent::tool_input_available("call_1", "search", json!({})),
            UIStreamEvent::tool_output_available("call_1", json!({})),
            UIStreamEvent::finish_step(),
            // Done
            UIStreamEvent::finish(),
        ];

        // Count step events
        let start_step_count = events
            .iter()
            .filter(|e| matches!(e, UIStreamEvent::StartStep))
            .count();
        let finish_step_count = events
            .iter()
            .filter(|e| matches!(e, UIStreamEvent::FinishStep))
            .count();
        assert_eq!(start_step_count, 2);
        assert_eq!(finish_step_count, 2);
    }

    #[test]
    fn test_error_flow() {
        let events = [
            UIStreamEvent::message_start("msg_1"),
            UIStreamEvent::text_start("txt_1"),
            UIStreamEvent::text_delta("txt_1", "Starting..."),
            UIStreamEvent::error("Connection lost"),
        ];

        assert!(events.iter().any(
            |e| matches!(e, UIStreamEvent::Error { error_text } if error_text == "Connection lost")
        ));
    }

    #[test]
    fn test_abort_flow() {
        let events = [
            UIStreamEvent::message_start("msg_1"),
            UIStreamEvent::text_start("txt_1"),
            UIStreamEvent::text_delta("txt_1", "Processing..."),
            UIStreamEvent::abort("User cancelled"),
        ];

        assert!(events
            .iter()
            .any(|e| matches!(e, UIStreamEvent::Abort { reason } if reason.as_deref() == Some("User cancelled"))));
    }

    #[test]
    fn test_reasoning_flow() {
        let events = vec![
            UIStreamEvent::message_start("msg_1"),
            UIStreamEvent::reasoning_start("r_1"),
            UIStreamEvent::reasoning_delta("r_1", "Let me think about this..."),
            UIStreamEvent::reasoning_delta("r_1", " I should consider..."),
            UIStreamEvent::reasoning_end("r_1"),
            UIStreamEvent::text_start("txt_1"),
            UIStreamEvent::text_delta("txt_1", "The answer is 42."),
            UIStreamEvent::text_end("txt_1"),
            UIStreamEvent::finish(),
        ];

        // Verify reasoning events come before text events
        let reasoning_end_idx = events
            .iter()
            .position(|e| matches!(e, UIStreamEvent::ReasoningEnd { .. }))
            .unwrap();
        let text_start_idx = events
            .iter()
            .position(|e| matches!(e, UIStreamEvent::TextStart { .. }))
            .unwrap();
        assert!(reasoning_end_idx < text_start_idx);
    }

    // ========================================================================
    // Additional Coverage Tests
    // ========================================================================

    #[test]
    fn test_data_event_factory() {
        let event = UIStreamEvent::data("custom", json!({"key": "value"}));
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains(r#""type":"data-custom""#));
        assert!(json.contains(r#""key":"value""#));
    }

    #[test]
    fn test_ui_message_new() {
        let msg = UIMessage::new("msg_1", UIRole::System);
        assert_eq!(msg.id, "msg_1");
        assert_eq!(msg.role, UIRole::System);
        assert!(msg.parts.is_empty());
        assert!(msg.metadata.is_none());
    }

    #[test]
    fn test_ui_message_with_parts_iterator() {
        let parts = vec![
            UIMessagePart::Text {
                text: "Part 1".to_string(),
                state: Some(StreamState::Done),
            },
            UIMessagePart::Text {
                text: "Part 2".to_string(),
                state: Some(StreamState::Done),
            },
        ];
        let msg = UIMessage::assistant("msg_1").with_parts(parts);
        assert_eq!(msg.parts.len(), 2);
        assert_eq!(msg.text_content(), "Part 1Part 2");
    }

    #[test]
    fn test_source_url_part_serialization() {
        let part = UIMessagePart::SourceUrl {
            source_id: "src_1".to_string(),
            url: "https://example.com".to_string(),
            title: Some("Example".to_string()),
            provider_metadata: None,
        };
        let json = serde_json::to_string(&part).unwrap();
        assert!(json.contains(r#""type":"source-url""#));
        assert!(json.contains(r#""sourceId":"src_1""#));
    }

    #[test]
    fn test_source_document_part_serialization() {
        let part = UIMessagePart::SourceDocument {
            source_id: "src_1".to_string(),
            media_type: "application/pdf".to_string(),
            title: "My Document".to_string(),
            filename: Some("doc.pdf".to_string()),
            provider_metadata: Some(json!({"pages": 10})),
        };
        let json = serde_json::to_string(&part).unwrap();
        assert!(json.contains(r#""type":"source-document""#));
        assert!(json.contains(r#""providerMetadata""#));
    }

    #[test]
    fn test_file_part_serialization() {
        let part = UIMessagePart::File {
            url: "https://example.com/image.png".to_string(),
            media_type: "image/png".to_string(),
        };
        let json = serde_json::to_string(&part).unwrap();
        assert!(json.contains(r#""type":"file""#));
        assert!(json.contains(r#""url":"https://example.com/image.png""#));
    }

    #[test]
    fn test_data_part_serialization() {
        let part = UIMessagePart::Data {
            data_type: "data-custom".to_string(),
            data: json!({"custom": "data"}),
        };
        let json = serde_json::to_string(&part).unwrap();
        assert!(json.contains(r#""type":"data-custom""#));
    }

    #[test]
    fn test_reasoning_part_with_metadata() {
        let part = UIMessagePart::Reasoning {
            text: "Thinking deeply...".to_string(),
            state: Some(StreamState::Done),
            provider_metadata: Some(json!({"model": "deepseek"})),
        };
        let json = serde_json::to_string(&part).unwrap();
        assert!(json.contains(r#""providerMetadata""#));
        assert!(json.contains(r#""deepseek""#));
    }

    #[test]
    fn test_ui_role_serialization() {
        assert_eq!(
            serde_json::to_string(&UIRole::System).unwrap(),
            r#""system""#
        );
        assert_eq!(serde_json::to_string(&UIRole::User).unwrap(), r#""user""#);
        assert_eq!(
            serde_json::to_string(&UIRole::Assistant).unwrap(),
            r#""assistant""#
        );
    }

    #[test]
    fn test_stream_state_serialization() {
        assert_eq!(
            serde_json::to_string(&StreamState::Streaming).unwrap(),
            r#""streaming""#
        );
        assert_eq!(
            serde_json::to_string(&StreamState::Done).unwrap(),
            r#""done""#
        );
    }

    #[test]
    fn test_tool_state_serialization() {
        assert_eq!(
            serde_json::to_string(&ToolState::InputStreaming).unwrap(),
            r#""input-streaming""#
        );
        assert_eq!(
            serde_json::to_string(&ToolState::InputAvailable).unwrap(),
            r#""input-available""#
        );
        assert_eq!(
            serde_json::to_string(&ToolState::OutputAvailable).unwrap(),
            r#""output-available""#
        );
        assert_eq!(
            serde_json::to_string(&ToolState::OutputError).unwrap(),
            r#""output-error""#
        );
    }

    #[test]
    fn test_ui_message_text_content_with_non_text_parts() {
        let msg = UIMessage::assistant("msg_1")
            .with_text("Text 1")
            .with_part(UIMessagePart::Tool {
                tool_call_id: "call_1".to_string(),
                tool_name: "search".to_string(),
                state: ToolState::OutputAvailable,
                input: Some(json!({})),
                output: Some(json!({})),
                error_text: None,
            })
            .with_text("Text 2");

        // Should only get text content, not tool parts
        assert_eq!(msg.text_content(), "Text 1Text 2");
    }

    #[test]
    fn test_file_event_without_provider_metadata() {
        let event = UIStreamEvent::file("https://example.com/doc.pdf", "application/pdf");
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains(r#""type":"file""#));
        assert!(!json.contains("providerMetadata"));
    }

    #[test]
    fn test_source_document_without_filename() {
        let event = UIStreamEvent::source_document("src_1", "text/plain", "Notes", None);
        let json = serde_json::to_string(&event).unwrap();
        assert!(!json.contains("filename"));
    }

    // ========================================================================
    // AiSdkAdapter Tests
    // ========================================================================

    #[test]
    fn test_ai_sdk_adapter_new() {
        let adapter = AiSdkAdapter::new("run_12345678".to_string());
        assert!(adapter.message_id().starts_with("msg_run_1234"));
        assert_eq!(adapter.run_id(), "run_12345678");
        assert_eq!(adapter.text_id(), "txt_0");
    }

    #[test]
    fn test_ai_sdk_adapter_default() {
        let adapter = AiSdkAdapter::default();
        assert_eq!(adapter.run_id(), "default");
    }

    #[test]
    fn test_ai_sdk_adapter_text_delta() {
        use crate::stream::AgentEvent;

        let adapter = AiSdkAdapter::new("run_123".to_string());
        let event = AgentEvent::TextDelta {
            delta: "Hello".to_string(),
        };
        let outputs = adapter.to_json(&event);
        assert_eq!(outputs.len(), 1);
        assert!(outputs[0].contains("text-delta"));
        assert!(outputs[0].contains("Hello"));
    }

    #[test]
    fn test_ai_sdk_adapter_to_sse() {
        use crate::stream::AgentEvent;

        let adapter = AiSdkAdapter::new("run_123".to_string());
        let event = AgentEvent::TextDelta {
            delta: "Hello".to_string(),
        };
        let sse_lines = adapter.to_sse(&event);
        assert_eq!(sse_lines.len(), 1);
        assert!(sse_lines[0].starts_with("data: "));
        assert!(sse_lines[0].ends_with("\n\n"));
    }

    #[test]
    fn test_ai_sdk_adapter_to_ndjson() {
        use crate::stream::AgentEvent;

        let adapter = AiSdkAdapter::new("run_123".to_string());
        let event = AgentEvent::TextDelta {
            delta: "Hello".to_string(),
        };
        let ndjson_lines = adapter.to_ndjson(&event);
        assert_eq!(ndjson_lines.len(), 1);
        assert!(ndjson_lines[0].ends_with("\n"));
        assert!(!ndjson_lines[0].starts_with("data:"));
    }

    #[test]
    fn test_ai_sdk_adapter_tool_call() {
        use crate::stream::AgentEvent;

        let adapter = AiSdkAdapter::new("run_123".to_string());
        let event = AgentEvent::ToolCallStart {
            id: "call_1".to_string(),
            name: "search".to_string(),
        };
        let outputs = adapter.to_json(&event);
        assert!(outputs[0].contains("tool-input-start"));
    }

    #[test]
    fn test_ai_sdk_adapter_state_as_data() {
        use crate::stream::AgentEvent;

        let adapter = AiSdkAdapter::new("run_123".to_string());
        let event = AgentEvent::StateSnapshot {
            snapshot: json!({"key": "value"}),
        };
        let outputs = adapter.to_json(&event);
        assert!(outputs[0].contains("data-state-snapshot"));
    }

    #[test]
    fn test_ai_sdk_adapter_helper_methods() {
        let adapter = AiSdkAdapter::new("run_123".to_string());

        let msg_start = adapter.message_start();
        assert!(matches!(msg_start, UIStreamEvent::MessageStart { .. }));

        let txt_start = adapter.text_start();
        assert!(matches!(txt_start, UIStreamEvent::TextStart { .. }));

        let txt_end = adapter.text_end();
        assert!(matches!(txt_end, UIStreamEvent::TextEnd { .. }));

        let finish = adapter.finish();
        assert!(matches!(finish, UIStreamEvent::Finish { .. }));
    }

    // ========================================================================
    // AI SDK v6 Strict Schema Compliance Tests
    //
    // V6 uses z7.strictObject which rejects extra fields. These tests verify
    // our serialization produces exactly the fields v6 expects.
    // ========================================================================

    /// Helper: parse JSON and return its keys.
    fn json_keys(json_str: &str) -> Vec<String> {
        let v: serde_json::Value = serde_json::from_str(json_str).unwrap();
        v.as_object().unwrap().keys().cloned().collect()
    }

    #[test]
    fn test_v6_start_event_strict_schema() {
        // v6 schema: { type: "start", messageId?: string, messageMetadata?: unknown }
        let event = UIStreamEvent::message_start("msg_1");
        let json = serde_json::to_string(&event).unwrap();
        let keys = json_keys(&json);
        assert!(keys.contains(&"type".to_string()));
        assert!(keys.contains(&"messageId".to_string()));
        // Must use "start" not "message-start"
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["type"], "start");
        // No extra fields beyond type + messageId (messageMetadata skipped when None)
        assert!(keys.len() <= 3, "too many fields: {:?}", keys);
    }

    #[test]
    fn test_v6_text_start_strict_schema() {
        // v6 schema: { type: "text-start", id: string, providerMetadata?: ... }
        let event = UIStreamEvent::text_start("txt_1");
        let json = serde_json::to_string(&event).unwrap();
        let keys = json_keys(&json);
        assert_eq!(keys.len(), 2); // type + id only
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["type"], "text-start");
        assert_eq!(v["id"], "txt_1");
    }

    #[test]
    fn test_v6_text_delta_strict_schema() {
        // v6 schema: { type: "text-delta", id: string, delta: string, providerMetadata?: ... }
        let event = UIStreamEvent::text_delta("txt_1", "Hello");
        let json = serde_json::to_string(&event).unwrap();
        let keys = json_keys(&json);
        assert_eq!(keys.len(), 3); // type + id + delta
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["type"], "text-delta");
    }

    #[test]
    fn test_v6_finish_strict_schema() {
        // v6 schema: { type: "finish", finishReason?: enum, messageMetadata?: unknown }
        let event = UIStreamEvent::finish_with_reason("stop");
        let json = serde_json::to_string(&event).unwrap();
        let keys = json_keys(&json);
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["type"], "finish");
        assert_eq!(v["finishReason"], "stop");
        // No extra fields
        assert!(keys.len() <= 3, "too many fields: {:?}", keys);
    }

    #[test]
    fn test_v6_finish_without_reason_strict_schema() {
        let event = UIStreamEvent::finish();
        let json = serde_json::to_string(&event).unwrap();
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["type"], "finish");
        // finishReason and messageMetadata should be absent (skip_serializing_if)
        assert!(v.get("finishReason").is_none());
        assert!(v.get("messageMetadata").is_none());
    }

    #[test]
    fn test_v6_error_strict_schema() {
        // v6 schema: { type: "error", errorText: string }
        let event = UIStreamEvent::error("Something went wrong");
        let json = serde_json::to_string(&event).unwrap();
        let keys = json_keys(&json);
        assert_eq!(keys.len(), 2); // type + errorText
    }

    #[test]
    fn test_v6_tool_input_start_strict_schema() {
        // v6 schema: { type: "tool-input-start", toolCallId, toolName, providerExecuted?, providerMetadata?, dynamic?, title? }
        let event = UIStreamEvent::tool_input_start("call_1", "search");
        let json = serde_json::to_string(&event).unwrap();
        let keys = json_keys(&json);
        assert_eq!(keys.len(), 3); // type + toolCallId + toolName
    }

    #[test]
    fn test_v6_tool_input_delta_strict_schema() {
        // v6 schema: { type: "tool-input-delta", toolCallId, inputTextDelta }
        let event = UIStreamEvent::tool_input_delta("call_1", r#"{"q":"r"}"#);
        let json = serde_json::to_string(&event).unwrap();
        let keys = json_keys(&json);
        assert_eq!(keys.len(), 3); // type + toolCallId + inputTextDelta
    }

    #[test]
    fn test_v6_tool_output_available_strict_schema() {
        // v6 schema: { type: "tool-output-available", toolCallId, output, providerExecuted?, dynamic?, preliminary? }
        let event = UIStreamEvent::tool_output_available("call_1", json!({"result": "ok"}));
        let json = serde_json::to_string(&event).unwrap();
        let keys = json_keys(&json);
        assert_eq!(keys.len(), 3); // type + toolCallId + output
    }

    #[test]
    fn test_v6_abort_strict_schema() {
        // v6 schema: { type: "abort", reason?: string }
        let event = UIStreamEvent::abort("cancelled");
        let json = serde_json::to_string(&event).unwrap();
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["type"], "abort");
        assert_eq!(v["reason"], "cancelled");
    }

    #[test]
    fn test_v6_file_strict_schema() {
        // v6 schema: { type: "file", url, mediaType, providerMetadata? }
        // NOTE: v6 strict schema does NOT include "filename"
        let event = UIStreamEvent::file("https://example.com/f.png", "image/png");
        let json = serde_json::to_string(&event).unwrap();
        let keys = json_keys(&json);
        assert_eq!(keys.len(), 3); // type + url + mediaType
        assert!(!json.contains("filename"));
    }

    #[test]
    fn test_v6_start_step_strict_schema() {
        // v6 schema: { type: "start-step" }
        let event = UIStreamEvent::start_step();
        let json = serde_json::to_string(&event).unwrap();
        let keys = json_keys(&json);
        assert_eq!(keys.len(), 1); // type only
    }

    #[test]
    fn test_v6_finish_step_strict_schema() {
        // v6 schema: { type: "finish-step" }
        let event = UIStreamEvent::finish_step();
        let json = serde_json::to_string(&event).unwrap();
        let keys = json_keys(&json);
        assert_eq!(keys.len(), 1); // type only
    }

    #[test]
    fn test_v6_data_event_strict_schema() {
        // v6 schema: { type: /^data-.*/, id?: string, data: unknown, transient?: boolean }
        let event = UIStreamEvent::data("run-info", json!({"runId": "r1"}));
        let json = serde_json::to_string(&event).unwrap();
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["type"], "data-run-info");
        assert!(v.get("data").is_some());
    }

    #[test]
    fn test_v6_complete_stream_sequence() {
        // Simulate a complete v6-compliant stream sequence
        let events = vec![
            UIStreamEvent::message_start("msg_1"),
            UIStreamEvent::text_start("txt_0"),
            UIStreamEvent::text_delta("txt_0", "Hello "),
            UIStreamEvent::text_delta("txt_0", "World!"),
            UIStreamEvent::text_end("txt_0"),
            UIStreamEvent::finish_with_reason("stop"),
        ];

        // All events should serialize to valid JSON
        for event in &events {
            let json = serde_json::to_string(event).unwrap();
            let v: serde_json::Value = serde_json::from_str(&json).unwrap();
            assert!(v["type"].is_string(), "event must have string type field");
        }

        // Verify the type sequence
        let types: Vec<String> = events
            .iter()
            .map(|e| {
                let json = serde_json::to_string(e).unwrap();
                let v: serde_json::Value = serde_json::from_str(&json).unwrap();
                v["type"].as_str().unwrap().to_string()
            })
            .collect();
        assert_eq!(
            types,
            vec!["start", "text-start", "text-delta", "text-delta", "text-end", "finish"]
        );
    }

    // ========================================================================
    // AiSdkEncoder Tests (stateful text lifecycle)
    // ========================================================================

    #[test]
    fn test_encoder_prologue_no_text_start() {
        let enc = AiSdkEncoder::new("run_1".to_string());
        let pro = enc.prologue();
        assert_eq!(pro.len(), 1);
        assert!(matches!(pro[0], UIStreamEvent::MessageStart { .. }));
    }

    #[test]
    fn test_encoder_text_delta_opens_text() {
        use crate::stream::AgentEvent;

        let mut enc = AiSdkEncoder::new("run_1".to_string());
        let out = enc.on_agent_event(&AgentEvent::TextDelta {
            delta: "hi".to_string(),
        });
        assert_eq!(out.len(), 2);
        assert!(matches!(out[0], UIStreamEvent::TextStart { .. }));
        assert!(matches!(out[1], UIStreamEvent::TextDelta { .. }));

        // Second text delta should NOT re-open
        let out2 = enc.on_agent_event(&AgentEvent::TextDelta {
            delta: " there".to_string(),
        });
        assert_eq!(out2.len(), 1);
        assert!(matches!(out2[0], UIStreamEvent::TextDelta { .. }));
    }

    #[test]
    fn test_encoder_tool_closes_text() {
        use crate::stream::AgentEvent;

        let mut enc = AiSdkEncoder::new("run_1".to_string());

        // Open text
        enc.on_agent_event(&AgentEvent::TextDelta {
            delta: "hi".to_string(),
        });

        // Tool call should close text
        let out = enc.on_agent_event(&AgentEvent::ToolCallStart {
            id: "tc-1".to_string(),
            name: "search".to_string(),
        });
        assert_eq!(out.len(), 2);
        assert!(matches!(out[0], UIStreamEvent::TextEnd { .. }));
        assert!(matches!(out[1], UIStreamEvent::ToolInputStart { .. }));
    }

    #[test]
    fn test_encoder_tool_without_text_no_text_end() {
        use crate::stream::AgentEvent;

        let mut enc = AiSdkEncoder::new("run_1".to_string());
        let out = enc.on_agent_event(&AgentEvent::ToolCallStart {
            id: "tc-1".to_string(),
            name: "search".to_string(),
        });
        assert_eq!(out.len(), 1);
        assert!(matches!(out[0], UIStreamEvent::ToolInputStart { .. }));
    }

    #[test]
    fn test_encoder_text_tool_text_increments_text_id() {
        use crate::stream::AgentEvent;

        let mut enc = AiSdkEncoder::new("run_1".to_string());

        // txt_0
        let out = enc.on_agent_event(&AgentEvent::TextDelta {
            delta: "a".to_string(),
        });
        let json = serde_json::to_string(&out[0]).unwrap();
        assert!(json.contains("txt_0"));

        // Tool closes txt_0
        enc.on_agent_event(&AgentEvent::ToolCallStart {
            id: "tc-1".to_string(),
            name: "x".to_string(),
        });

        // txt_1
        let out = enc.on_agent_event(&AgentEvent::TextDelta {
            delta: "b".to_string(),
        });
        let json = serde_json::to_string(&out[0]).unwrap();
        assert!(json.contains("txt_1"));
    }

    #[test]
    fn test_encoder_finish_closes_text() {
        use crate::stream::AgentEvent;

        let mut enc = AiSdkEncoder::new("run_1".to_string());
        enc.on_agent_event(&AgentEvent::TextDelta {
            delta: "hi".to_string(),
        });

        let out = enc.on_agent_event(&AgentEvent::RunFinish {
            thread_id: "t".to_string(),
            run_id: "run_1".to_string(),
            result: None,
            stop_reason: None,
        });
        assert_eq!(out.len(), 2);
        assert!(matches!(out[0], UIStreamEvent::TextEnd { .. }));
        assert!(matches!(out[1], UIStreamEvent::Finish { .. }));
    }

    #[test]
    fn test_encoder_finish_without_text() {
        use crate::stream::AgentEvent;

        let mut enc = AiSdkEncoder::new("run_1".to_string());
        let out = enc.on_agent_event(&AgentEvent::RunFinish {
            thread_id: "t".to_string(),
            run_id: "run_1".to_string(),
            result: None,
            stop_reason: None,
        });
        assert_eq!(out.len(), 1);
        assert!(matches!(out[0], UIStreamEvent::Finish { .. }));
    }

    #[test]
    fn test_encoder_error_no_text_end() {
        use crate::stream::AgentEvent;

        let mut enc = AiSdkEncoder::new("run_1".to_string());
        enc.on_agent_event(&AgentEvent::TextDelta {
            delta: "hi".to_string(),
        });

        let out = enc.on_agent_event(&AgentEvent::Error {
            message: "boom".to_string(),
        });
        assert_eq!(out.len(), 1);
        assert!(matches!(out[0], UIStreamEvent::Error { .. }));

        // After error, nothing
        let out = enc.on_agent_event(&AgentEvent::TextDelta {
            delta: "x".to_string(),
        });
        assert!(out.is_empty());
    }

    #[test]
    fn test_encoder_ignores_after_finish() {
        use crate::stream::AgentEvent;

        let mut enc = AiSdkEncoder::new("run_1".to_string());
        enc.on_agent_event(&AgentEvent::RunFinish {
            thread_id: "t".to_string(),
            run_id: "run_1".to_string(),
            result: None,
            stop_reason: None,
        });

        let out = enc.on_agent_event(&AgentEvent::TextDelta {
            delta: "late".to_string(),
        });
        assert!(out.is_empty());
    }
}
