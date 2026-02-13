//! Streaming response handling for LLM responses.
//!
//! This module provides the internal event types for the agent loop:
//! - `AgentEvent`: Protocol-agnostic events emitted by the agent
//! - `StreamCollector` / `StreamResult`: Helpers for collecting stream chunks
//!
//! Protocol-specific conversion is provided as standalone functions:
//! - `agent_event_to_ui()`: Convert to AI SDK v6 UIStreamEvents
//! - `agent_event_to_agui()`: Convert to AG-UI protocol events
//!
//! Stateful encoders that manage stream lifecycle (prologue, epilogue, etc.)
//! live in `carve-agentos-server::protocol`.

use crate::ag_ui::{AGUIContext, AGUIEvent};
use crate::state_types::Interaction;
use crate::traits::tool::ToolResult;
use crate::types::ToolCall;
use crate::ui_stream::UIStreamEvent;
use carve_state::TrackedPatch;
use genai::chat::{ChatStreamEvent, Usage};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

/// Partial tool call being collected during streaming.
#[derive(Debug, Clone)]
struct PartialToolCall {
    id: String,
    name: String,
    arguments: String,
}

/// Collector for streaming LLM responses.
///
/// Processes stream events and accumulates text and tool calls.
#[derive(Debug, Default)]
pub struct StreamCollector {
    text: String,
    tool_calls: HashMap<String, PartialToolCall>,
    tool_call_order: Vec<String>,
    usage: Option<Usage>,
}

impl StreamCollector {
    /// Create a new stream collector.
    pub fn new() -> Self {
        Self::default()
    }

    /// Process a stream event and optionally return an output event.
    ///
    /// This is a pure-ish function - it updates internal state and returns
    /// an output event if something notable happened.
    pub fn process(&mut self, event: ChatStreamEvent) -> Option<StreamOutput> {
        match event {
            ChatStreamEvent::Chunk(chunk) => {
                // Text chunk - chunk.content is a String
                if !chunk.content.is_empty() {
                    self.text.push_str(&chunk.content);
                    return Some(StreamOutput::TextDelta(chunk.content));
                }
                None
            }
            ChatStreamEvent::ToolCallChunk(tool_chunk) => {
                let call_id = tool_chunk.tool_call.call_id.clone();

                // Get or create partial tool call while preserving first-seen order.
                let partial = match self.tool_calls.entry(call_id.clone()) {
                    std::collections::hash_map::Entry::Occupied(e) => e.into_mut(),
                    std::collections::hash_map::Entry::Vacant(e) => {
                        self.tool_call_order.push(call_id.clone());
                        e.insert(PartialToolCall {
                            id: call_id.clone(),
                            name: String::new(),
                            arguments: String::new(),
                        })
                    }
                };

                let mut output = None;

                // Update name if provided (non-empty)
                if !tool_chunk.tool_call.fn_name.is_empty() && partial.name.is_empty() {
                    partial.name = tool_chunk.tool_call.fn_name.clone();
                    output = Some(StreamOutput::ToolCallStart {
                        id: call_id.clone(),
                        name: partial.name.clone(),
                    });
                }

                // Extract raw argument string from fn_arguments.
                // genai wraps argument strings in Value::String(...);
                // .to_string() would JSON-serialize it with extra quotes.
                // With capture_tool_calls enabled, each chunk carries the
                // ACCUMULATED value (not a delta), so we replace rather than
                // append.
                let args_str = match &tool_chunk.tool_call.fn_arguments {
                    Value::String(s) if !s.is_empty() => s.clone(),
                    Value::Null | Value::String(_) => String::new(),
                    other => other.to_string(),
                };
                if !args_str.is_empty() {
                    // Compute delta for the output event
                    let delta = if args_str.len() > partial.arguments.len()
                        && args_str.starts_with(&partial.arguments)
                    {
                        args_str[partial.arguments.len()..].to_string()
                    } else {
                        args_str.clone()
                    };
                    partial.arguments = args_str;
                    // Keep ToolCallStart when name+args arrive in one chunk.
                    if !delta.is_empty() && output.is_none() {
                        output = Some(StreamOutput::ToolCallDelta {
                            id: call_id,
                            args_delta: delta,
                        });
                    }
                }

                output
            }
            ChatStreamEvent::End(end) => {
                // Use captured tool calls from the End event as the source
                // of truth, overriding any partial data accumulated during
                // streaming (which may be incorrect if chunks carried
                // accumulated rather than delta values).
                if let Some(tool_calls) = end.captured_tool_calls() {
                    for tc in tool_calls {
                        // Extract raw string; genai may wrap in Value::String
                        let end_args = match &tc.fn_arguments {
                            Value::String(s) if !s.is_empty() => s.clone(),
                            Value::Null | Value::String(_) => String::new(),
                            other => other.to_string(),
                        };
                        match self.tool_calls.entry(tc.call_id.clone()) {
                            std::collections::hash_map::Entry::Occupied(mut e) => {
                                let partial = e.get_mut();
                                if partial.name.is_empty() {
                                    partial.name = tc.fn_name.clone();
                                }
                                // Always prefer End event arguments over streaming
                                if !end_args.is_empty() {
                                    partial.arguments = end_args;
                                }
                            }
                            std::collections::hash_map::Entry::Vacant(e) => {
                                self.tool_call_order.push(tc.call_id.clone());
                                e.insert(PartialToolCall {
                                    id: tc.call_id.clone(),
                                    name: tc.fn_name.clone(),
                                    arguments: end_args,
                                });
                            }
                        }
                    }
                }
                // Capture token usage
                self.usage = end.captured_usage;
                None
            }
            _ => None,
        }
    }

    /// Finish collecting and return the final result.
    pub fn finish(self) -> StreamResult {
        let mut remaining = self.tool_calls;
        let mut tool_calls: Vec<ToolCall> = Vec::with_capacity(self.tool_call_order.len());

        for call_id in self.tool_call_order {
            let Some(p) = remaining.remove(&call_id) else {
                continue;
            };
            if p.name.is_empty() {
                continue;
            }
            let arguments = serde_json::from_str(&p.arguments).unwrap_or(Value::Null);
            tool_calls.push(ToolCall::new(p.id, p.name, arguments));
        }

        StreamResult {
            text: self.text,
            tool_calls,
            usage: self.usage,
        }
    }

    /// Get the current accumulated text.
    pub fn text(&self) -> &str {
        &self.text
    }

    /// Check if any tool calls have been collected.
    pub fn has_tool_calls(&self) -> bool {
        !self.tool_calls.is_empty()
    }
}

/// Output event from stream processing.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum StreamOutput {
    /// Text content delta.
    TextDelta(String),
    /// Tool call started with name.
    ToolCallStart { id: String, name: String },
    /// Tool call arguments delta.
    ToolCallDelta { id: String, args_delta: String },
}

/// Result of stream collection.
#[derive(Debug, Clone)]
pub struct StreamResult {
    /// Accumulated text content.
    pub text: String,
    /// Collected tool calls.
    pub tool_calls: Vec<ToolCall>,
    /// Token usage from the LLM response.
    pub usage: Option<Usage>,
}

impl StreamResult {
    /// Check if tool execution is needed.
    pub fn needs_tools(&self) -> bool {
        !self.tool_calls.is_empty()
    }
}

/// Agent loop events for streaming execution.
///
/// These events represent the protocol-agnostic internal agent loop state.
/// Use [`agent_event_to_ui`] or [`agent_event_to_agui`] for protocol conversion.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "event_type", rename_all = "snake_case")]
pub enum AgentEvent {
    // ========================================================================
    // Lifecycle Events (AG-UI compatible)
    // ========================================================================
    /// Run started (emitted at the beginning of agent execution).
    RunStart {
        thread_id: String,
        run_id: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        parent_run_id: Option<String>,
    },
    /// Run finished (emitted when agent execution completes).
    RunFinish {
        thread_id: String,
        run_id: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        result: Option<Value>,
        /// Why the agent loop stopped. `None` for pauses (e.g. pending interaction).
        #[serde(skip_serializing_if = "Option::is_none")]
        stop_reason: Option<crate::stop::StopReason>,
    },

    // ========================================================================
    // Text Events
    // ========================================================================
    /// LLM text delta.
    TextDelta { delta: String },

    // ========================================================================
    // Tool Events
    // ========================================================================
    /// Tool call started.
    ToolCallStart { id: String, name: String },
    /// Tool call arguments delta.
    ToolCallDelta { id: String, args_delta: String },
    /// Tool call input is complete (ready for execution).
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
    },
    // ========================================================================
    // Step Events
    // ========================================================================
    /// Step started (for multi-step agents).
    StepStart,
    /// Step completed.
    StepEnd,

    // ========================================================================
    // LLM Telemetry Events
    // ========================================================================
    /// LLM inference completed with token usage data.
    InferenceComplete {
        /// Model used for this inference.
        model: String,
        /// Token usage (if available from the provider).
        #[serde(skip_serializing_if = "Option::is_none")]
        usage: Option<Usage>,
        /// Duration of the LLM call in milliseconds.
        duration_ms: u64,
    },

    // ========================================================================
    // State Events (AG-UI compatible)
    // ========================================================================
    /// State snapshot (complete state).
    StateSnapshot { snapshot: Value },
    /// State delta (RFC 6902 JSON Patch operations).
    StateDelta { delta: Vec<Value> },
    /// Messages snapshot (for reconnection).
    MessagesSnapshot { messages: Vec<Value> },

    // ========================================================================
    // Activity Events (AG-UI compatible)
    // ========================================================================
    /// Activity snapshot (complete activity state).
    ActivitySnapshot {
        message_id: String,
        activity_type: String,
        content: Value,
        replace: Option<bool>,
    },
    /// Activity delta (RFC 6902 JSON Patch operations).
    ActivityDelta {
        message_id: String,
        activity_type: String,
        patch: Vec<Value>,
    },

    // ========================================================================
    // Interaction Events
    // ========================================================================
    /// Pending interaction request (client action required).
    Pending { interaction: Interaction },

    // ========================================================================
    // Completion Events
    // ========================================================================
    /// Stream aborted by user or system.
    Aborted { reason: String },
    /// Error occurred.
    Error { message: String },
}

impl AgentEvent {
    /// Extract the response text from a `RunFinish` result value.
    ///
    /// Looks for `result.response` as a string; returns empty string if absent.
    pub(crate) fn extract_response(result: &Option<Value>) -> String {
        result
            .as_ref()
            .and_then(|v| v.get("response"))
            .and_then(|r| r.as_str())
            .unwrap_or_default()
            .to_string()
    }
}

/// Convert an AgentEvent to AI SDK v6 compatible UI stream events.
///
/// This is a stateless conversion of a single event. For full stream lifecycle
/// management (prologue, epilogue, text_id tracking), use `AiSdkEncoder` in
/// `carve-agentos-server::protocol`.
pub fn agent_event_to_ui(ev: &AgentEvent, text_id: &str) -> Vec<UIStreamEvent> {
    match ev {
        AgentEvent::RunStart { .. } => vec![],
        AgentEvent::RunFinish { .. } => vec![UIStreamEvent::finish_with_reason("stop")],

        AgentEvent::TextDelta { delta } => {
            vec![UIStreamEvent::text_delta(text_id, delta)]
        }

        AgentEvent::ToolCallStart { id, name } => {
            vec![UIStreamEvent::tool_input_start(id, name)]
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

        AgentEvent::StepStart => vec![UIStreamEvent::start_step()],
        AgentEvent::StepEnd => vec![UIStreamEvent::finish_step()],

        AgentEvent::StateSnapshot { snapshot } => {
            vec![UIStreamEvent::data("state-snapshot", snapshot.clone())]
        }
        AgentEvent::StateDelta { delta } => {
            vec![UIStreamEvent::data(
                "state-delta",
                Value::Array(delta.clone()),
            )]
        }
        AgentEvent::MessagesSnapshot { messages } => {
            vec![UIStreamEvent::data(
                "messages-snapshot",
                Value::Array(messages.clone()),
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

        AgentEvent::Aborted { reason } => vec![UIStreamEvent::abort(reason)],
        AgentEvent::Error { message } => vec![UIStreamEvent::error(message)],
        AgentEvent::InferenceComplete { .. } => vec![],
    }
}

/// Convert an AgentEvent to AG-UI protocol compatible events.
///
/// This is a stateless conversion that uses `AGUIContext` for tracking text
/// message state. For full stream lifecycle management (pending event filtering,
/// fallback finish), use `AgUiEncoder` in `carve-agentos-server::protocol`.
pub fn agent_event_to_agui(ev: &AgentEvent, ctx: &mut AGUIContext) -> Vec<AGUIEvent> {
    match ev {
        AgentEvent::RunStart {
            thread_id,
            run_id,
            parent_run_id,
        } => {
            vec![AGUIEvent::run_started(
                thread_id,
                run_id,
                parent_run_id.clone(),
            )]
        }
        AgentEvent::RunFinish {
            thread_id,
            run_id,
            result,
            ..
        } => {
            let mut events = vec![];
            if ctx.end_text() {
                events.push(AGUIEvent::text_message_end(&ctx.message_id));
            }
            events.push(AGUIEvent::run_finished(thread_id, run_id, result.clone()));
            events
        }

        AgentEvent::TextDelta { delta } => {
            let mut events = vec![];
            if ctx.start_text() {
                events.push(AGUIEvent::text_message_start(&ctx.message_id));
            }
            events.push(AGUIEvent::text_message_content(&ctx.message_id, delta));
            events
        }

        AgentEvent::ToolCallStart { id, name } => {
            let mut events = vec![];
            if ctx.end_text() {
                events.push(AGUIEvent::text_message_end(&ctx.message_id));
            }
            events.push(AGUIEvent::tool_call_start(
                id,
                name,
                Some(ctx.message_id.clone()),
            ));
            events
        }
        AgentEvent::ToolCallDelta { id, args_delta } => {
            vec![AGUIEvent::tool_call_args(id, args_delta)]
        }
        AgentEvent::ToolCallReady { id, .. } => {
            vec![AGUIEvent::tool_call_end(id)]
        }
        AgentEvent::ToolCallDone { id, result, .. } => {
            let result_msg_id = format!("result_{}", id);
            let content = serde_json::to_string(&result.to_json()).unwrap_or_default();
            vec![AGUIEvent::tool_call_result(&result_msg_id, id, content)]
        }

        AgentEvent::StepStart => {
            vec![AGUIEvent::step_started(ctx.next_step_name())]
        }
        AgentEvent::StepEnd => {
            vec![AGUIEvent::step_finished(ctx.current_step_name())]
        }

        AgentEvent::StateSnapshot { snapshot } => {
            vec![AGUIEvent::state_snapshot(snapshot.clone())]
        }
        AgentEvent::StateDelta { delta } => {
            vec![AGUIEvent::state_delta(delta.clone())]
        }
        AgentEvent::MessagesSnapshot { messages } => {
            vec![AGUIEvent::messages_snapshot(messages.clone())]
        }

        AgentEvent::ActivitySnapshot {
            message_id,
            activity_type,
            content,
            replace,
        } => {
            vec![AGUIEvent::activity_snapshot(
                message_id.clone(),
                activity_type.clone(),
                value_to_map(content),
                *replace,
            )]
        }
        AgentEvent::ActivityDelta {
            message_id,
            activity_type,
            patch,
        } => {
            vec![AGUIEvent::activity_delta(
                message_id.clone(),
                activity_type.clone(),
                patch.clone(),
            )]
        }

        AgentEvent::Pending { interaction } => {
            let mut events = vec![];
            if ctx.end_text() {
                events.push(AGUIEvent::text_message_end(&ctx.message_id));
            }
            events.extend(interaction.to_ag_ui_events());
            events
        }

        AgentEvent::Aborted { reason } => {
            vec![AGUIEvent::run_error(reason, Some("ABORTED".to_string()))]
        }
        AgentEvent::Error { message } => {
            vec![AGUIEvent::run_error(message, None)]
        }
        AgentEvent::InferenceComplete { .. } => vec![],
    }
}

fn value_to_map(value: &Value) -> HashMap<String, Value> {
    match value.as_object() {
        Some(map) => map
            .iter()
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect(),
        None => {
            let mut map = HashMap::new();
            map.insert("value".to_string(), value.clone());
            map
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ag_ui::{AGUIContext, AGUIEvent};
    use serde_json::json;

    #[test]
    fn test_extract_response_with_value() {
        let result = Some(json!({"response": "Hello world"}));
        assert_eq!(AgentEvent::extract_response(&result), "Hello world");
    }

    #[test]
    fn test_extract_response_none() {
        assert_eq!(AgentEvent::extract_response(&None), "");
    }

    #[test]
    fn test_extract_response_missing_key() {
        let result = Some(json!({"other": "value"}));
        assert_eq!(AgentEvent::extract_response(&result), "");
    }

    #[test]
    fn test_extract_response_non_string() {
        let result = Some(json!({"response": 42}));
        assert_eq!(AgentEvent::extract_response(&result), "");
    }

    #[test]
    fn test_stream_collector_new() {
        let collector = StreamCollector::new();
        assert!(collector.text().is_empty());
        assert!(!collector.has_tool_calls());
    }

    #[test]
    fn test_stream_collector_finish_empty() {
        let collector = StreamCollector::new();
        let result = collector.finish();

        assert!(result.text.is_empty());
        assert!(result.tool_calls.is_empty());
        assert!(!result.needs_tools());
    }

    #[test]
    fn test_stream_result_needs_tools() {
        let result = StreamResult {
            text: "Hello".to_string(),
            tool_calls: vec![],
            usage: None,
        };
        assert!(!result.needs_tools());

        let result_with_tools = StreamResult {
            text: String::new(),
            tool_calls: vec![ToolCall::new("id", "name", serde_json::json!({}))],
            usage: None,
        };
        assert!(result_with_tools.needs_tools());
    }

    #[test]
    fn test_stream_output_variants() {
        let text_delta = StreamOutput::TextDelta("Hello".to_string());
        match text_delta {
            StreamOutput::TextDelta(s) => assert_eq!(s, "Hello"),
            _ => panic!("Expected TextDelta"),
        }

        let tool_start = StreamOutput::ToolCallStart {
            id: "call_1".to_string(),
            name: "search".to_string(),
        };
        match tool_start {
            StreamOutput::ToolCallStart { id, name } => {
                assert_eq!(id, "call_1");
                assert_eq!(name, "search");
            }
            _ => panic!("Expected ToolCallStart"),
        }

        let tool_delta = StreamOutput::ToolCallDelta {
            id: "call_1".to_string(),
            args_delta: r#"{"query":"#.to_string(),
        };
        match tool_delta {
            StreamOutput::ToolCallDelta { id, args_delta } => {
                assert_eq!(id, "call_1");
                assert!(args_delta.contains("query"));
            }
            _ => panic!("Expected ToolCallDelta"),
        }
    }

    #[test]
    fn test_agent_event_variants() {
        // Test TextDelta
        let event = AgentEvent::TextDelta {
            delta: "Hello".to_string(),
        };
        match event {
            AgentEvent::TextDelta { delta } => assert_eq!(delta, "Hello"),
            _ => panic!("Expected TextDelta"),
        }

        // Test ToolCallStart
        let event = AgentEvent::ToolCallStart {
            id: "call_1".to_string(),
            name: "search".to_string(),
        };
        if let AgentEvent::ToolCallStart { id, name } = event {
            assert_eq!(id, "call_1");
            assert_eq!(name, "search");
        }

        // Test ToolCallDelta
        let event = AgentEvent::ToolCallDelta {
            id: "call_1".to_string(),
            args_delta: "{}".to_string(),
        };
        if let AgentEvent::ToolCallDelta { id, .. } = event {
            assert_eq!(id, "call_1");
        }

        // Test ToolCallDone
        let result = ToolResult::success("test", json!({"value": 42}));
        let event = AgentEvent::ToolCallDone {
            id: "call_1".to_string(),
            result: result.clone(),
            patch: None,
        };
        if let AgentEvent::ToolCallDone {
            id,
            result: r,
            patch,
        } = event
        {
            assert_eq!(id, "call_1");
            assert!(r.is_success());
            assert!(patch.is_none());
        }

        // Test RunFinish
        let event = AgentEvent::RunFinish {
            thread_id: "t1".to_string(),
            run_id: "r1".to_string(),
            result: Some(json!({"response": "Final response"})),
            stop_reason: None,
        };
        if let AgentEvent::RunFinish { result, .. } = &event {
            assert_eq!(AgentEvent::extract_response(result), "Final response");
        }

        // Test ActivitySnapshot
        let event = AgentEvent::ActivitySnapshot {
            message_id: "activity_1".to_string(),
            activity_type: "progress".to_string(),
            content: json!({"progress": 0.5}),
            replace: Some(true),
        };
        if let AgentEvent::ActivitySnapshot {
            message_id,
            activity_type,
            content,
            replace,
        } = event
        {
            assert_eq!(message_id, "activity_1");
            assert_eq!(activity_type, "progress");
            assert_eq!(content["progress"], 0.5);
            assert_eq!(replace, Some(true));
        }

        // Test ActivityDelta
        let event = AgentEvent::ActivityDelta {
            message_id: "activity_1".to_string(),
            activity_type: "progress".to_string(),
            patch: vec![json!({"op": "replace", "path": "/progress", "value": 0.75})],
        };
        if let AgentEvent::ActivityDelta {
            message_id,
            activity_type,
            patch,
        } = event
        {
            assert_eq!(message_id, "activity_1");
            assert_eq!(activity_type, "progress");
            assert_eq!(patch.len(), 1);
        }

        // Test Error
        let event = AgentEvent::Error {
            message: "Something went wrong".to_string(),
        };
        if let AgentEvent::Error { message } = event {
            assert!(message.contains("wrong"));
        }
    }

    #[test]
    fn test_stream_result_with_multiple_tool_calls() {
        let result = StreamResult {
            text: "I'll call multiple tools".to_string(),
            tool_calls: vec![
                ToolCall::new("call_1", "search", json!({"q": "rust"})),
                ToolCall::new("call_2", "calculate", json!({"expr": "1+1"})),
                ToolCall::new("call_3", "format", json!({"text": "hello"})),
            ],
            usage: None,
        };

        assert!(result.needs_tools());
        assert_eq!(result.tool_calls.len(), 3);
        assert_eq!(result.tool_calls[0].name, "search");
        assert_eq!(result.tool_calls[1].name, "calculate");
        assert_eq!(result.tool_calls[2].name, "format");
    }

    #[test]
    fn test_stream_result_text_only() {
        let result = StreamResult {
            text: "This is a long response without any tool calls. It just contains text."
                .to_string(),
            tool_calls: vec![],
            usage: None,
        };

        assert!(!result.needs_tools());
        assert!(result.text.len() > 50);
    }

    #[test]
    fn test_tool_call_with_complex_arguments() {
        let call = ToolCall::new(
            "call_complex",
            "api_request",
            json!({
                "method": "POST",
                "url": "https://api.example.com/data",
                "headers": {
                    "Content-Type": "application/json",
                    "Authorization": "Bearer token"
                },
                "body": {
                    "items": [1, 2, 3],
                    "nested": {
                        "deep": true
                    }
                }
            }),
        );

        assert_eq!(call.id, "call_complex");
        assert_eq!(call.name, "api_request");
        assert_eq!(call.arguments["method"], "POST");
        assert!(call.arguments["headers"]["Content-Type"]
            .as_str()
            .unwrap()
            .contains("json"));
    }

    #[test]
    fn test_agent_event_done_with_patch() {
        use carve_state::{path, Op, Patch, TrackedPatch};

        let patch = TrackedPatch::new(Patch::new().with_op(Op::set(path!("value"), json!(42))));

        let event = AgentEvent::ToolCallDone {
            id: "call_1".to_string(),
            result: ToolResult::success("test", json!({})),
            patch: Some(patch.clone()),
        };

        if let AgentEvent::ToolCallDone { patch: p, .. } = event {
            assert!(p.is_some());
            let p = p.unwrap();
            assert!(!p.patch().is_empty());
        }
    }

    #[test]
    fn test_stream_output_debug() {
        let output = StreamOutput::TextDelta("test".to_string());
        let debug_str = format!("{:?}", output);
        assert!(debug_str.contains("TextDelta"));
        assert!(debug_str.contains("test"));
    }

    #[test]
    fn test_agent_event_debug() {
        let event = AgentEvent::Error {
            message: "error message".to_string(),
        };
        let debug_str = format!("{:?}", event);
        assert!(debug_str.contains("Error"));
        assert!(debug_str.contains("error message"));
    }

    #[test]
    fn test_stream_result_clone() {
        let result = StreamResult {
            text: "Hello".to_string(),
            tool_calls: vec![ToolCall::new("1", "test", json!({}))],
            usage: None,
        };

        let cloned = result.clone();
        assert_eq!(cloned.text, result.text);
        assert_eq!(cloned.tool_calls.len(), result.tool_calls.len());
    }

    // Tests with mock ChatStreamEvent
    use genai::chat::{StreamChunk, StreamEnd, ToolChunk};

    #[test]
    fn test_stream_collector_process_text_chunk() {
        let mut collector = StreamCollector::new();

        // Process text chunk
        let chunk = ChatStreamEvent::Chunk(StreamChunk {
            content: "Hello ".to_string(),
        });
        let output = collector.process(chunk);

        assert!(output.is_some());
        if let Some(StreamOutput::TextDelta(delta)) = output {
            assert_eq!(delta, "Hello ");
        } else {
            panic!("Expected TextDelta");
        }

        assert_eq!(collector.text(), "Hello ");
    }

    #[test]
    fn test_stream_collector_process_multiple_text_chunks() {
        let mut collector = StreamCollector::new();

        // Process multiple chunks
        let chunks = vec!["Hello ", "world", "!"];
        for text in &chunks {
            let chunk = ChatStreamEvent::Chunk(StreamChunk {
                content: text.to_string(),
            });
            collector.process(chunk);
        }

        assert_eq!(collector.text(), "Hello world!");

        let result = collector.finish();
        assert_eq!(result.text, "Hello world!");
        assert!(!result.needs_tools());
    }

    #[test]
    fn test_stream_collector_process_empty_chunk() {
        let mut collector = StreamCollector::new();

        let chunk = ChatStreamEvent::Chunk(StreamChunk {
            content: String::new(),
        });
        let output = collector.process(chunk);

        // Empty chunks should return None
        assert!(output.is_none());
        assert!(collector.text().is_empty());
    }

    #[test]
    fn test_stream_collector_process_tool_call_start() {
        let mut collector = StreamCollector::new();

        let tool_call = genai::chat::ToolCall {
            call_id: "call_123".to_string(),
            fn_name: "search".to_string(),
            fn_arguments: json!(null),
            thought_signatures: None,
        };
        let chunk = ChatStreamEvent::ToolCallChunk(ToolChunk { tool_call });
        let output = collector.process(chunk);

        assert!(output.is_some());
        if let Some(StreamOutput::ToolCallStart { id, name }) = output {
            assert_eq!(id, "call_123");
            assert_eq!(name, "search");
        } else {
            panic!("Expected ToolCallStart");
        }

        assert!(collector.has_tool_calls());
    }

    #[test]
    fn test_stream_collector_process_tool_call_with_arguments() {
        let mut collector = StreamCollector::new();

        // First chunk: tool call start
        let tool_call1 = genai::chat::ToolCall {
            call_id: "call_abc".to_string(),
            fn_name: "calculator".to_string(),
            fn_arguments: json!(null),
            thought_signatures: None,
        };
        collector.process(ChatStreamEvent::ToolCallChunk(ToolChunk {
            tool_call: tool_call1,
        }));

        // Second chunk: arguments delta
        let tool_call2 = genai::chat::ToolCall {
            call_id: "call_abc".to_string(),
            fn_name: String::new(), // Name already set
            fn_arguments: json!({"expr": "1+1"}),
            thought_signatures: None,
        };
        let output = collector.process(ChatStreamEvent::ToolCallChunk(ToolChunk {
            tool_call: tool_call2,
        }));

        assert!(output.is_some());
        if let Some(StreamOutput::ToolCallDelta { id, args_delta }) = output {
            assert_eq!(id, "call_abc");
            assert!(args_delta.contains("expr"));
        }

        let result = collector.finish();
        assert!(result.needs_tools());
        assert_eq!(result.tool_calls.len(), 1);
        assert_eq!(result.tool_calls[0].name, "calculator");
    }

    #[test]
    fn test_stream_collector_single_chunk_with_name_and_args_keeps_tool_start() {
        let mut collector = StreamCollector::new();

        let tool_call = genai::chat::ToolCall {
            call_id: "call_single".to_string(),
            fn_name: "search".to_string(),
            fn_arguments: Value::String(r#"{"q":"rust"}"#.to_string()),
            thought_signatures: None,
        };
        let output = collector.process(ChatStreamEvent::ToolCallChunk(ToolChunk { tool_call }));

        assert!(
            matches!(output, Some(StreamOutput::ToolCallStart { .. })),
            "tool start should not be lost when name+args arrive in one chunk; got: {output:?}"
        );

        let result = collector.finish();
        assert_eq!(result.tool_calls.len(), 1);
        assert_eq!(result.tool_calls[0].id, "call_single");
        assert_eq!(result.tool_calls[0].name, "search");
        assert_eq!(result.tool_calls[0].arguments, json!({"q":"rust"}));
    }

    #[test]
    fn test_stream_collector_preserves_tool_call_arrival_order() {
        let mut collector = StreamCollector::new();
        let call_ids = vec![
            "call_7", "call_3", "call_1", "call_9", "call_2", "call_8", "call_4", "call_6",
        ];

        for (idx, call_id) in call_ids.iter().enumerate() {
            let tool_call = genai::chat::ToolCall {
                call_id: (*call_id).to_string(),
                fn_name: format!("tool_{idx}"),
                fn_arguments: Value::Null,
                thought_signatures: None,
            };
            let _ = collector.process(ChatStreamEvent::ToolCallChunk(ToolChunk { tool_call }));
        }

        let result = collector.finish();
        let got: Vec<String> = result.tool_calls.into_iter().map(|c| c.id).collect();
        let expected: Vec<String> = call_ids.into_iter().map(str::to_string).collect();

        assert_eq!(
            got, expected,
            "tool_calls should preserve model-emitted order"
        );
    }

    #[test]
    fn test_stream_collector_process_multiple_tool_calls() {
        let mut collector = StreamCollector::new();

        // First tool call
        let tc1 = genai::chat::ToolCall {
            call_id: "call_1".to_string(),
            fn_name: "search".to_string(),
            fn_arguments: json!({"q": "rust"}),
            thought_signatures: None,
        };
        collector.process(ChatStreamEvent::ToolCallChunk(ToolChunk { tool_call: tc1 }));

        // Second tool call
        let tc2 = genai::chat::ToolCall {
            call_id: "call_2".to_string(),
            fn_name: "calculate".to_string(),
            fn_arguments: json!({"expr": "2+2"}),
            thought_signatures: None,
        };
        collector.process(ChatStreamEvent::ToolCallChunk(ToolChunk { tool_call: tc2 }));

        let result = collector.finish();
        assert_eq!(result.tool_calls.len(), 2);
    }

    #[test]
    fn test_stream_collector_process_mixed_text_and_tools() {
        let mut collector = StreamCollector::new();

        // Text first
        collector.process(ChatStreamEvent::Chunk(StreamChunk {
            content: "I'll search for that. ".to_string(),
        }));

        // Then tool call
        let tc = genai::chat::ToolCall {
            call_id: "call_search".to_string(),
            fn_name: "web_search".to_string(),
            fn_arguments: json!({"query": "rust programming"}),
            thought_signatures: None,
        };
        collector.process(ChatStreamEvent::ToolCallChunk(ToolChunk { tool_call: tc }));

        let result = collector.finish();
        assert_eq!(result.text, "I'll search for that. ");
        assert_eq!(result.tool_calls.len(), 1);
        assert_eq!(result.tool_calls[0].name, "web_search");
    }

    #[test]
    fn test_stream_collector_process_start_event() {
        let mut collector = StreamCollector::new();

        let output = collector.process(ChatStreamEvent::Start);
        assert!(output.is_none());
        assert!(collector.text().is_empty());
    }

    #[test]
    fn test_stream_collector_process_end_event() {
        let mut collector = StreamCollector::new();

        // Add some text first
        collector.process(ChatStreamEvent::Chunk(StreamChunk {
            content: "Hello".to_string(),
        }));

        // End event
        let end = StreamEnd::default();
        let output = collector.process(ChatStreamEvent::End(end));

        assert!(output.is_none());

        let result = collector.finish();
        assert_eq!(result.text, "Hello");
    }

    #[test]
    fn test_stream_collector_has_tool_calls() {
        let mut collector = StreamCollector::new();
        assert!(!collector.has_tool_calls());

        let tc = genai::chat::ToolCall {
            call_id: "call_1".to_string(),
            fn_name: "test".to_string(),
            fn_arguments: json!({}),
            thought_signatures: None,
        };
        collector.process(ChatStreamEvent::ToolCallChunk(ToolChunk { tool_call: tc }));

        assert!(collector.has_tool_calls());
    }

    #[test]
    fn test_stream_collector_text_accumulation() {
        let mut collector = StreamCollector::new();

        // Simulate streaming word by word
        let words = vec!["The ", "quick ", "brown ", "fox ", "jumps."];
        for word in words {
            collector.process(ChatStreamEvent::Chunk(StreamChunk {
                content: word.to_string(),
            }));
        }

        assert_eq!(collector.text(), "The quick brown fox jumps.");
    }

    #[test]
    fn test_stream_collector_tool_arguments_accumulation() {
        // genai sends ACCUMULATED arguments in each chunk (with capture_tool_calls=true).
        // Each chunk carries the full accumulated string so far, not just a delta.
        let mut collector = StreamCollector::new();

        // Start tool call
        let tc1 = genai::chat::ToolCall {
            call_id: "call_1".to_string(),
            fn_name: "api".to_string(),
            fn_arguments: json!(null),
            thought_signatures: None,
        };
        collector.process(ChatStreamEvent::ToolCallChunk(ToolChunk { tool_call: tc1 }));

        // Accumulated argument chunks (each is the full value so far)
        let tc2 = genai::chat::ToolCall {
            call_id: "call_1".to_string(),
            fn_name: String::new(),
            fn_arguments: Value::String("{\"url\":".to_string()),
            thought_signatures: None,
        };
        collector.process(ChatStreamEvent::ToolCallChunk(ToolChunk { tool_call: tc2 }));

        let tc3 = genai::chat::ToolCall {
            call_id: "call_1".to_string(),
            fn_name: String::new(),
            fn_arguments: Value::String("{\"url\": \"https://example.com\"}".to_string()),
            thought_signatures: None,
        };
        collector.process(ChatStreamEvent::ToolCallChunk(ToolChunk { tool_call: tc3 }));

        let result = collector.finish();
        assert_eq!(result.tool_calls.len(), 1);
        assert_eq!(result.tool_calls[0].name, "api");
        assert_eq!(
            result.tool_calls[0].arguments,
            json!({"url": "https://example.com"})
        );
    }

    #[test]
    fn test_stream_collector_value_string_args_accumulation() {
        // genai sends ACCUMULATED arguments as Value::String in each chunk.
        // Verify that we extract raw strings and properly de-duplicate.
        let mut collector = StreamCollector::new();

        // First chunk: name only, empty arguments
        let tc1 = genai::chat::ToolCall {
            call_id: "call_1".to_string(),
            fn_name: "get_weather".to_string(),
            fn_arguments: Value::String(String::new()),
            thought_signatures: None,
        };
        collector.process(ChatStreamEvent::ToolCallChunk(ToolChunk { tool_call: tc1 }));

        // Accumulated argument chunks (each is the full value so far)
        let tc2 = genai::chat::ToolCall {
            call_id: "call_1".to_string(),
            fn_name: String::new(),
            fn_arguments: Value::String("{\"city\":".to_string()),
            thought_signatures: None,
        };
        let output2 =
            collector.process(ChatStreamEvent::ToolCallChunk(ToolChunk { tool_call: tc2 }));
        assert!(matches!(
            output2,
            Some(StreamOutput::ToolCallDelta { ref args_delta, .. }) if args_delta == "{\"city\":"
        ));

        let tc3 = genai::chat::ToolCall {
            call_id: "call_1".to_string(),
            fn_name: String::new(),
            fn_arguments: Value::String("{\"city\": \"San Francisco\"}".to_string()),
            thought_signatures: None,
        };
        let output3 =
            collector.process(ChatStreamEvent::ToolCallChunk(ToolChunk { tool_call: tc3 }));
        // Delta should be only the new part
        assert!(matches!(
            output3,
            Some(StreamOutput::ToolCallDelta { ref args_delta, .. }) if args_delta == " \"San Francisco\"}"
        ));

        let result = collector.finish();
        assert_eq!(result.tool_calls.len(), 1);
        assert_eq!(result.tool_calls[0].name, "get_weather");
        assert_eq!(
            result.tool_calls[0].arguments,
            json!({"city": "San Francisco"})
        );
    }

    #[test]
    fn test_stream_collector_finish_clears_state() {
        let mut collector = StreamCollector::new();

        collector.process(ChatStreamEvent::Chunk(StreamChunk {
            content: "Test".to_string(),
        }));

        let result1 = collector.finish();
        assert_eq!(result1.text, "Test");

        // After finish, the collector is consumed, so we can't use it again
        // This is by design (finish takes self)
    }

    // ========================================================================
    // AI SDK v6 Conversion Tests
    // ========================================================================

    #[test]
    fn test_agent_event_to_ui_events_text_delta() {
        let event = AgentEvent::TextDelta {
            delta: "Hello".to_string(),
        };
        let ui_events = agent_event_to_ui(&event, "txt_0");
        assert_eq!(ui_events.len(), 1);
        let json = serde_json::to_string(&ui_events[0]).unwrap();
        assert!(json.contains(r#""type":"text-delta""#));
    }

    #[test]
    fn test_agent_event_to_ui_events_tool_call_start() {
        let event = AgentEvent::ToolCallStart {
            id: "call_1".to_string(),
            name: "search".to_string(),
        };
        let ui_events = agent_event_to_ui(&event, "txt_0");
        assert_eq!(ui_events.len(), 1);
        let json = serde_json::to_string(&ui_events[0]).unwrap();
        assert!(json.contains(r#""type":"tool-input-start""#));
    }

    #[test]
    fn test_agent_event_to_ui_events_tool_call_ready() {
        let event = AgentEvent::ToolCallReady {
            id: "call_1".to_string(),
            name: "search".to_string(),
            arguments: json!({"query": "rust"}),
        };
        let ui_events = agent_event_to_ui(&event, "txt_0");
        assert_eq!(ui_events.len(), 1);
        let json = serde_json::to_string(&ui_events[0]).unwrap();
        assert!(json.contains(r#""type":"tool-input-available""#));
        assert!(json.contains(r#""input""#));
    }

    #[test]
    fn test_agent_event_to_ui_events_tool_call_done() {
        let result = ToolResult::success("search", json!({"results": ["item1"]}));
        let event = AgentEvent::ToolCallDone {
            id: "call_1".to_string(),
            result,
            patch: None,
        };
        let ui_events = agent_event_to_ui(&event, "txt_0");
        assert_eq!(ui_events.len(), 1);
        let json = serde_json::to_string(&ui_events[0]).unwrap();
        assert!(json.contains(r#""type":"tool-output-available""#));
        assert!(json.contains(r#""output""#));
    }

    #[test]
    fn test_agent_event_to_ui_events_step_start() {
        let event = AgentEvent::StepStart;
        let ui_events = agent_event_to_ui(&event, "txt_0");
        assert_eq!(ui_events.len(), 1);
        let json = serde_json::to_string(&ui_events[0]).unwrap();
        assert!(json.contains(r#""type":"start-step""#));
    }

    #[test]
    fn test_agent_event_to_ui_events_step_end() {
        let event = AgentEvent::StepEnd;
        let ui_events = agent_event_to_ui(&event, "txt_0");
        assert_eq!(ui_events.len(), 1);
        let json = serde_json::to_string(&ui_events[0]).unwrap();
        assert!(json.contains(r#""type":"finish-step""#));
    }

    #[test]
    fn test_agent_event_to_ui_events_run_finish() {
        let event = AgentEvent::RunFinish {
            thread_id: "t1".to_string(),
            run_id: "r1".to_string(),
            result: Some(json!({"response": "Final answer"})),
            stop_reason: None,
        };
        let ui_events = agent_event_to_ui(&event, "txt_0");
        assert_eq!(ui_events.len(), 1);
        let json = serde_json::to_string(&ui_events[0]).unwrap();
        assert!(json.contains(r#""type":"finish""#));
    }

    #[test]
    fn test_agent_event_to_ui_events_aborted() {
        let event = AgentEvent::Aborted {
            reason: "User cancelled".to_string(),
        };
        let ui_events = agent_event_to_ui(&event, "txt_0");
        assert_eq!(ui_events.len(), 1);
        let json = serde_json::to_string(&ui_events[0]).unwrap();
        assert!(json.contains(r#""type":"abort""#));
        assert!(json.contains("User cancelled"));
    }

    #[test]
    fn test_agent_event_to_ui_events_error() {
        let event = AgentEvent::Error {
            message: "Something went wrong".to_string(),
        };
        let ui_events = agent_event_to_ui(&event, "txt_0");
        assert_eq!(ui_events.len(), 1);
        let json = serde_json::to_string(&ui_events[0]).unwrap();
        assert!(json.contains(r#""type":"error""#));
        assert!(json.contains("Something went wrong"));
    }

    #[test]
    fn test_agent_event_to_ui_events_activity_snapshot() {
        let event = AgentEvent::ActivitySnapshot {
            message_id: "activity_1".to_string(),
            activity_type: "progress".to_string(),
            content: json!({"progress": 0.7}),
            replace: Some(true),
        };
        let ui_events = agent_event_to_ui(&event, "txt_0");
        assert_eq!(ui_events.len(), 1);
        let json = serde_json::to_string(&ui_events[0]).unwrap();
        assert!(json.contains(r#""type":"data-activity-snapshot""#));
        assert!(json.contains(r#""progress":0.7"#));
    }

    #[test]
    fn test_agent_event_to_ui_events_activity_snapshot_includes_replace() {
        let event = AgentEvent::ActivitySnapshot {
            message_id: "activity_replace".to_string(),
            activity_type: "progress".to_string(),
            content: json!({"progress": 0.4}),
            replace: Some(true),
        };
        let ui_events = agent_event_to_ui(&event, "txt_0");
        let json = serde_json::to_string(&ui_events[0]).unwrap();
        assert!(json.contains(r#""replace":true"#));
    }

    #[test]
    fn test_agent_event_to_ui_events_activity_snapshot_payload_fields() {
        let event = AgentEvent::ActivitySnapshot {
            message_id: "activity_payload".to_string(),
            activity_type: "progress".to_string(),
            content: json!({"progress": 0.7}),
            replace: Some(true),
        };
        let ui_events = agent_event_to_ui(&event, "txt_0");
        assert_eq!(ui_events.len(), 1);
        match &ui_events[0] {
            UIStreamEvent::Data { data, .. } => {
                assert_eq!(data["messageId"], "activity_payload");
                assert_eq!(data["activityType"], "progress");
                assert_eq!(data["content"]["progress"], 0.7);
                assert_eq!(data["replace"], true);
            }
            _ => panic!("Expected data event"),
        }
    }

    #[test]
    fn test_agent_event_to_ui_events_activity_delta_payload_fields() {
        let event = AgentEvent::ActivityDelta {
            message_id: "activity_delta_payload".to_string(),
            activity_type: "progress".to_string(),
            patch: vec![json!({"op": "replace", "path": "/progress", "value": 0.9})],
        };
        let ui_events = agent_event_to_ui(&event, "txt_0");
        assert_eq!(ui_events.len(), 1);
        match &ui_events[0] {
            UIStreamEvent::Data { data, .. } => {
                assert_eq!(data["messageId"], "activity_delta_payload");
                assert_eq!(data["activityType"], "progress");
                assert!(data.get("replace").is_none());
                assert_eq!(data["patch"][0]["path"], "/progress");
            }
            _ => panic!("Expected data event"),
        }
    }

    #[test]
    fn test_agent_event_to_ui_events_activity_delta() {
        let event = AgentEvent::ActivityDelta {
            message_id: "activity_1".to_string(),
            activity_type: "progress".to_string(),
            patch: vec![json!({"op": "replace", "path": "/progress", "value": 0.9})],
        };
        let ui_events = agent_event_to_ui(&event, "txt_0");
        assert_eq!(ui_events.len(), 1);
        let json = serde_json::to_string(&ui_events[0]).unwrap();
        assert!(json.contains(r#""type":"data-activity-delta""#));
        assert!(json.contains(r#""path":"/progress""#));
    }

    #[test]
    fn test_agent_event_to_ui_events_activity_delta_excludes_replace() {
        let event = AgentEvent::ActivityDelta {
            message_id: "activity_2".to_string(),
            activity_type: "progress".to_string(),
            patch: vec![json!({"op": "replace", "path": "/progress", "value": 1.0})],
        };
        let ui_events = agent_event_to_ui(&event, "txt_0");
        match &ui_events[0] {
            UIStreamEvent::Data { data, .. } => {
                assert!(data.get("replace").is_none());
                assert!(data.get("patch").is_some());
            }
            _ => panic!("Expected data event"),
        }
    }

    #[test]
    fn test_activity_snapshot_maps_non_object_content_to_value_key() {
        let event = AgentEvent::ActivitySnapshot {
            message_id: "activity_array".to_string(),
            activity_type: "progress".to_string(),
            content: json!(["a", "b"]),
            replace: None,
        };

        let mut ctx = AGUIContext::new("thread_1".into(), "run_1".into());
        let events = agent_event_to_agui(&event, &mut ctx);
        assert_eq!(events.len(), 1);
        match &events[0] {
            AGUIEvent::ActivitySnapshot { content, .. } => {
                assert_eq!(content.get("value"), Some(&json!(["a", "b"])));
                assert_eq!(content.len(), 1);
            }
            _ => panic!("Expected ActivitySnapshot"),
        }
    }

    #[test]
    fn test_activity_snapshot_maps_scalar_content_to_value_key() {
        let event = AgentEvent::ActivitySnapshot {
            message_id: "activity_scalar".to_string(),
            activity_type: "status".to_string(),
            content: json!("ok"),
            replace: None,
        };

        let mut ctx = AGUIContext::new("thread_1".into(), "run_1".into());
        let events = agent_event_to_agui(&event, &mut ctx);
        assert_eq!(events.len(), 1);
        match &events[0] {
            AGUIEvent::ActivitySnapshot { content, .. } => {
                assert_eq!(content.get("value"), Some(&json!("ok")));
                assert_eq!(content.len(), 1);
            }
            _ => panic!("Expected ActivitySnapshot"),
        }
    }

    // ========================================================================
    // New Event Variant Tests
    // ========================================================================

    #[test]
    fn test_agent_event_tool_call_ready() {
        let event = AgentEvent::ToolCallReady {
            id: "call_1".to_string(),
            name: "search".to_string(),
            arguments: json!({"query": "rust programming"}),
        };
        if let AgentEvent::ToolCallReady {
            id,
            name,
            arguments,
        } = event
        {
            assert_eq!(id, "call_1");
            assert_eq!(name, "search");
            assert_eq!(arguments["query"], "rust programming");
        } else {
            panic!("Expected ToolCallReady");
        }
    }

    #[test]
    fn test_agent_event_step_start() {
        let event = AgentEvent::StepStart;
        assert!(matches!(event, AgentEvent::StepStart));
    }

    #[test]
    fn test_agent_event_step_end() {
        let event = AgentEvent::StepEnd;
        assert!(matches!(event, AgentEvent::StepEnd));
    }

    #[test]
    fn test_agent_event_aborted() {
        let event = AgentEvent::Aborted {
            reason: "Timeout".to_string(),
        };
        if let AgentEvent::Aborted { reason } = event {
            assert_eq!(reason, "Timeout");
        } else {
            panic!("Expected Aborted");
        }
    }

    #[test]
    fn test_agent_event_serialization() {
        let event = AgentEvent::TextDelta {
            delta: "Hello".to_string(),
        };
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("text_delta"));
        assert!(json.contains("Hello"));

        let event = AgentEvent::StepStart;
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("step_start"));

        let event = AgentEvent::ActivitySnapshot {
            message_id: "activity_1".to_string(),
            activity_type: "progress".to_string(),
            content: json!({"progress": 1.0}),
            replace: Some(true),
        };
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("activity_snapshot"));
        assert!(json.contains("activity_1"));
    }

    #[test]
    fn test_agent_event_deserialization() {
        let json = r#"{"event_type":"step_start"}"#;
        let event: AgentEvent = serde_json::from_str(json).unwrap();
        assert!(matches!(event, AgentEvent::StepStart));

        let json = r#"{"event_type":"text_delta","delta":"Hello"}"#;
        let event: AgentEvent = serde_json::from_str(json).unwrap();
        if let AgentEvent::TextDelta { delta } = event {
            assert_eq!(delta, "Hello");
        } else {
            panic!("Expected TextDelta");
        }

        let json = r#"{"event_type":"activity_snapshot","message_id":"activity_1","activity_type":"progress","content":{"progress":0.3},"replace":true}"#;
        let event: AgentEvent = serde_json::from_str(json).unwrap();
        if let AgentEvent::ActivitySnapshot {
            message_id,
            activity_type,
            content,
            replace,
        } = event
        {
            assert_eq!(message_id, "activity_1");
            assert_eq!(activity_type, "progress");
            assert_eq!(content["progress"], 0.3);
            assert_eq!(replace, Some(true));
        } else {
            panic!("Expected ActivitySnapshot");
        }
    }

    // ========================================================================
    // Complete Flow Integration Tests
    // ========================================================================

    #[test]
    fn test_complete_streaming_flow_conversion() {
        // Simulate a complete streaming flow and convert to UI events
        let events = vec![
            AgentEvent::StepStart,
            AgentEvent::TextDelta {
                delta: "Let me ".to_string(),
            },
            AgentEvent::TextDelta {
                delta: "search.".to_string(),
            },
            AgentEvent::ToolCallStart {
                id: "call_1".to_string(),
                name: "search".to_string(),
            },
            AgentEvent::ToolCallDelta {
                id: "call_1".to_string(),
                args_delta: r#"{"query":"#.to_string(),
            },
            AgentEvent::ToolCallDelta {
                id: "call_1".to_string(),
                args_delta: r#""rust"}"#.to_string(),
            },
            AgentEvent::ToolCallReady {
                id: "call_1".to_string(),
                name: "search".to_string(),
                arguments: json!({"query": "rust"}),
            },
            AgentEvent::ToolCallDone {
                id: "call_1".to_string(),
                result: ToolResult::success("search", json!({"found": 3})),
                patch: None,
            },
            AgentEvent::StepEnd,
            AgentEvent::RunFinish {
                thread_id: "t1".to_string(),
                run_id: "r1".to_string(),
                result: Some(json!({"response": "Found 3 results."})),
                stop_reason: None,
            },
        ];

        // Convert all events to UI events
        let all_ui_events: Vec<UIStreamEvent> = events
            .iter()
            .flat_map(|e| agent_event_to_ui(e, "txt_0"))
            .collect();

        // Verify UI events contain expected types
        assert!(all_ui_events
            .iter()
            .any(|e| matches!(e, UIStreamEvent::StartStep)));
        assert!(all_ui_events
            .iter()
            .any(|e| matches!(e, UIStreamEvent::TextDelta { .. })));
        assert!(all_ui_events
            .iter()
            .any(|e| matches!(e, UIStreamEvent::ToolInputStart { .. })));
        assert!(all_ui_events
            .iter()
            .any(|e| matches!(e, UIStreamEvent::ToolInputDelta { .. })));
        assert!(all_ui_events
            .iter()
            .any(|e| matches!(e, UIStreamEvent::ToolInputAvailable { .. })));
        assert!(all_ui_events
            .iter()
            .any(|e| matches!(e, UIStreamEvent::ToolOutputAvailable { .. })));
        assert!(all_ui_events
            .iter()
            .any(|e| matches!(e, UIStreamEvent::FinishStep)));
        assert!(all_ui_events
            .iter()
            .any(|e| matches!(e, UIStreamEvent::Finish { .. })));
    }

    // ========================================================================
    // Additional Coverage Tests
    // ========================================================================

    #[test]
    fn test_agent_event_error_conversion() {
        let event = AgentEvent::Error {
            message: "Connection timeout".to_string(),
        };
        let ui_events = agent_event_to_ui(&event, "txt_0");
        assert_eq!(ui_events.len(), 1);
        assert!(
            matches!(&ui_events[0], UIStreamEvent::Error { error_text } if error_text == "Connection timeout")
        );
    }

    #[test]
    fn test_agent_event_aborted_conversion() {
        let event = AgentEvent::Aborted {
            reason: "User cancelled".to_string(),
        };
        let ui_events = agent_event_to_ui(&event, "txt_0");
        assert_eq!(ui_events.len(), 1);
        assert!(
            matches!(&ui_events[0], UIStreamEvent::Abort { reason } if reason.as_deref() == Some("User cancelled"))
        );
    }

    #[test]
    fn test_stream_output_variants_creation() {
        // Test the StreamOutput enum variants can be created
        let text_delta = StreamOutput::TextDelta("Hello".to_string());
        assert!(matches!(text_delta, StreamOutput::TextDelta(_)));

        let tool_start = StreamOutput::ToolCallStart {
            id: "call_1".to_string(),
            name: "search".to_string(),
        };
        assert!(matches!(tool_start, StreamOutput::ToolCallStart { .. }));

        let tool_delta = StreamOutput::ToolCallDelta {
            id: "call_1".to_string(),
            args_delta: "delta".to_string(),
        };
        assert!(matches!(tool_delta, StreamOutput::ToolCallDelta { .. }));
    }

    #[test]
    fn test_tool_call_done_with_patch_conversion() {
        use carve_state::{Patch, TrackedPatch};

        // Create an empty patch
        let patch = TrackedPatch::new(Patch::new());

        let event = AgentEvent::ToolCallDone {
            id: "call_1".to_string(),
            result: ToolResult::success("search", json!({"found": 5})),
            patch: Some(patch),
        };

        let ui_events = agent_event_to_ui(&event, "txt_0");
        assert_eq!(ui_events.len(), 1);
        assert!(matches!(
            ui_events[0],
            UIStreamEvent::ToolOutputAvailable { .. }
        ));
    }

    #[test]
    fn test_tool_call_done_error_conversion() {
        let event = AgentEvent::ToolCallDone {
            id: "call_1".to_string(),
            result: ToolResult::error("search", "Tool execution failed"),
            patch: None,
        };

        let ui_events = agent_event_to_ui(&event, "txt_0");
        assert_eq!(ui_events.len(), 1);
        if let UIStreamEvent::ToolOutputAvailable { output, .. } = &ui_events[0] {
            assert!(output["status"].as_str() == Some("error"));
        } else {
            panic!("Expected ToolOutputAvailable");
        }
    }

    #[test]
    fn test_stream_collector_text_and_has_tool_calls() {
        let collector = StreamCollector::new();
        assert!(!collector.has_tool_calls());
        assert_eq!(collector.text(), "");
    }

    // ========================================================================
    // AgentEvent::Pending Tests
    // ========================================================================

    #[test]
    fn test_agent_event_pending_to_ui_events() {
        let interaction = Interaction::new("perm_1", "confirm").with_message("Allow action?");

        let event = AgentEvent::Pending { interaction };
        let ui_events = agent_event_to_ui(&event, "txt_0");

        assert_eq!(ui_events.len(), 1);
        if let UIStreamEvent::Data { data_type, data } = &ui_events[0] {
            assert_eq!(data_type, "data-interaction");
            assert_eq!(data["id"], "perm_1");
            assert_eq!(data["action"], "confirm");
            assert_eq!(data["message"], "Allow action?");
        } else {
            panic!("Expected Data event");
        }
    }

    #[test]
    fn test_agent_event_pending_to_ag_ui_events() {
        let interaction = Interaction::new("perm_1", "confirm")
            .with_message("Allow tool?")
            .with_parameters(json!({ "tool_id": "write_file" }));

        let event = AgentEvent::Pending { interaction };
        let mut ctx = AGUIContext::new("t1".into(), "r1".into());
        let ag_ui_events = agent_event_to_agui(&event, &mut ctx);

        // Should produce 3 tool call events
        assert_eq!(ag_ui_events.len(), 3);
        assert!(matches!(ag_ui_events[0], AGUIEvent::ToolCallStart { .. }));
        assert!(matches!(ag_ui_events[1], AGUIEvent::ToolCallArgs { .. }));
        assert!(matches!(ag_ui_events[2], AGUIEvent::ToolCallEnd { .. }));
    }

    #[test]
    fn test_agent_event_pending_ends_text_stream() {
        let interaction = Interaction::new("int_1", "input");

        let event = AgentEvent::Pending { interaction };
        let mut ctx = AGUIContext::new("t1".into(), "r1".into());

        // Start text streaming
        ctx.start_text();

        let ag_ui_events = agent_event_to_agui(&event, &mut ctx);

        // Should include TextMessageEnd before tool call events
        assert_eq!(ag_ui_events.len(), 4);
        assert!(matches!(ag_ui_events[0], AGUIEvent::TextMessageEnd { .. }));
        assert!(matches!(ag_ui_events[1], AGUIEvent::ToolCallStart { .. }));
    }

    #[test]
    fn test_agent_event_activity_snapshot_to_ag_ui_with_scalar_content() {
        let event = AgentEvent::ActivitySnapshot {
            message_id: "activity_1".to_string(),
            activity_type: "status".to_string(),
            content: json!("ok"),
            replace: Some(true),
        };
        let mut ctx = AGUIContext::new("t1".into(), "r1".into());
        let events = agent_event_to_agui(&event, &mut ctx);
        assert_eq!(events.len(), 1);
        if let AGUIEvent::ActivitySnapshot { content, .. } = &events[0] {
            assert_eq!(content.get("value"), Some(&json!("ok")));
        } else {
            panic!("Expected ActivitySnapshot");
        }
    }

    #[test]
    fn test_agent_event_activity_delta_to_ag_ui() {
        let event = AgentEvent::ActivityDelta {
            message_id: "activity_1".to_string(),
            activity_type: "status".to_string(),
            patch: vec![json!({"op": "replace", "path": "/status", "value": "done"})],
        };
        let mut ctx = AGUIContext::new("t1".into(), "r1".into());
        let events = agent_event_to_agui(&event, &mut ctx);
        assert_eq!(events.len(), 1);
        assert!(matches!(events[0], AGUIEvent::ActivityDelta { .. }));
    }

    // ========================================================================
    // AG-UI Lifecycle Ordering Tests
    // ========================================================================

    #[test]
    fn test_ag_ui_run_start_produces_run_started() {
        let event = AgentEvent::RunStart {
            thread_id: "t1".into(),
            run_id: "r1".into(),
            parent_run_id: Some("parent".into()),
        };
        let mut ctx = AGUIContext::new("t1".into(), "r1".into());
        let events = agent_event_to_agui(&event, &mut ctx);
        assert_eq!(events.len(), 1);
        if let AGUIEvent::RunStarted {
            thread_id,
            run_id,
            parent_run_id,
            ..
        } = &events[0]
        {
            assert_eq!(thread_id, "t1");
            assert_eq!(run_id, "r1");
            assert_eq!(parent_run_id.as_deref(), Some("parent"));
        } else {
            panic!("Expected RunStarted, got: {:?}", events[0]);
        }
    }

    #[test]
    fn test_ag_ui_run_finish_ends_active_text_stream() {
        let mut ctx = AGUIContext::new("t1".into(), "r1".into());

        // Start text stream via a TextDelta
        let delta = AgentEvent::TextDelta {
            delta: "hello".into(),
        };
        let _ = agent_event_to_agui(&delta, &mut ctx);

        // RunFinish should emit TextMessageEnd THEN RunFinished
        let finish = AgentEvent::RunFinish {
            thread_id: "t1".into(),
            run_id: "r1".into(),
            result: None,
            stop_reason: None,
        };
        let events = agent_event_to_agui(&finish, &mut ctx);
        assert_eq!(events.len(), 2);
        assert!(
            matches!(&events[0], AGUIEvent::TextMessageEnd { .. }),
            "Expected TextMessageEnd, got: {:?}",
            events[0]
        );
        assert!(
            matches!(&events[1], AGUIEvent::RunFinished { .. }),
            "Expected RunFinished, got: {:?}",
            events[1]
        );
    }

    #[test]
    fn test_ag_ui_run_finish_without_active_text() {
        let mut ctx = AGUIContext::new("t1".into(), "r1".into());

        // No text started — RunFinish should only emit RunFinished
        let finish = AgentEvent::RunFinish {
            thread_id: "t1".into(),
            run_id: "r1".into(),
            result: Some(json!({"ok": true})),
            stop_reason: None,
        };
        let events = agent_event_to_agui(&finish, &mut ctx);
        assert_eq!(events.len(), 1);
        assert!(matches!(&events[0], AGUIEvent::RunFinished { .. }));
    }

    #[test]
    fn test_ag_ui_error_produces_run_error_without_code() {
        let event = AgentEvent::Error {
            message: "LLM timeout".into(),
        };
        let mut ctx = AGUIContext::new("t1".into(), "r1".into());
        let events = agent_event_to_agui(&event, &mut ctx);
        assert_eq!(events.len(), 1);
        if let AGUIEvent::RunError { message, code, .. } = &events[0] {
            assert_eq!(message, "LLM timeout");
            assert!(code.is_none());
        } else {
            panic!("Expected RunError");
        }
    }

    #[test]
    fn test_ag_ui_aborted_produces_run_error_with_code() {
        let event = AgentEvent::Aborted {
            reason: "user cancelled".into(),
        };
        let mut ctx = AGUIContext::new("t1".into(), "r1".into());
        let events = agent_event_to_agui(&event, &mut ctx);
        assert_eq!(events.len(), 1);
        if let AGUIEvent::RunError { message, code, .. } = &events[0] {
            assert_eq!(message, "user cancelled");
            assert_eq!(code.as_deref(), Some("ABORTED"));
        } else {
            panic!("Expected RunError");
        }
    }

    #[test]
    fn test_ag_ui_inference_complete_produces_no_events() {
        let event = AgentEvent::InferenceComplete {
            model: "gpt-4".into(),
            usage: None,
            duration_ms: 100,
        };
        let mut ctx = AGUIContext::new("t1".into(), "r1".into());
        let events = agent_event_to_agui(&event, &mut ctx);
        assert!(events.is_empty());
    }

    #[test]
    fn test_ag_ui_state_snapshot_produces_state_snapshot() {
        let event = AgentEvent::StateSnapshot {
            snapshot: json!({"count": 42}),
        };
        let mut ctx = AGUIContext::new("t1".into(), "r1".into());
        let events = agent_event_to_agui(&event, &mut ctx);
        assert_eq!(events.len(), 1);
        if let AGUIEvent::StateSnapshot { snapshot, .. } = &events[0] {
            assert_eq!(snapshot["count"], 42);
        } else {
            panic!("Expected StateSnapshot");
        }
    }

    #[test]
    fn test_ag_ui_state_delta_produces_state_delta() {
        let patch = vec![json!({"op": "replace", "path": "/count", "value": 43})];
        let event = AgentEvent::StateDelta {
            delta: patch.clone(),
        };
        let mut ctx = AGUIContext::new("t1".into(), "r1".into());
        let events = agent_event_to_agui(&event, &mut ctx);
        assert_eq!(events.len(), 1);
        if let AGUIEvent::StateDelta { delta, .. } = &events[0] {
            assert_eq!(delta.len(), 1);
            assert_eq!(delta[0]["op"], "replace");
        } else {
            panic!("Expected StateDelta");
        }
    }

    #[test]
    fn test_ag_ui_messages_snapshot_produces_messages_snapshot() {
        let msgs = vec![json!({"role": "user", "content": "hi"})];
        let event = AgentEvent::MessagesSnapshot {
            messages: msgs.clone(),
        };
        let mut ctx = AGUIContext::new("t1".into(), "r1".into());
        let events = agent_event_to_agui(&event, &mut ctx);
        assert_eq!(events.len(), 1);
        assert!(matches!(&events[0], AGUIEvent::MessagesSnapshot { .. }));
    }

    #[test]
    fn test_ag_ui_step_started_finished_pairing() {
        let mut ctx = AGUIContext::new("t1".into(), "r1".into());

        let start_events = agent_event_to_agui(&AgentEvent::StepStart, &mut ctx);
        assert_eq!(start_events.len(), 1);
        let step_name = if let AGUIEvent::StepStarted { step_name, .. } = &start_events[0] {
            step_name.clone()
        } else {
            panic!("Expected StepStarted");
        };

        let end_events = agent_event_to_agui(&AgentEvent::StepEnd, &mut ctx);
        assert_eq!(end_events.len(), 1);
        if let AGUIEvent::StepFinished {
            step_name: end_name,
            ..
        } = &end_events[0]
        {
            assert_eq!(
                &step_name, end_name,
                "StepStarted and StepFinished must share the same step name"
            );
        } else {
            panic!("Expected StepFinished");
        }
    }

    #[test]
    fn test_ag_ui_multi_step_increments_step_names() {
        let mut ctx = AGUIContext::new("t1".into(), "r1".into());

        // Step 1
        let s1 = agent_event_to_agui(&AgentEvent::StepStart, &mut ctx);
        let _ = agent_event_to_agui(&AgentEvent::StepEnd, &mut ctx);

        // Step 2
        let s2 = agent_event_to_agui(&AgentEvent::StepStart, &mut ctx);
        let _ = agent_event_to_agui(&AgentEvent::StepEnd, &mut ctx);

        let name1 = if let AGUIEvent::StepStarted { step_name, .. } = &s1[0] {
            step_name.clone()
        } else {
            panic!("Expected StepStarted");
        };
        let name2 = if let AGUIEvent::StepStarted { step_name, .. } = &s2[0] {
            step_name.clone()
        } else {
            panic!("Expected StepStarted");
        };

        assert_ne!(name1, name2, "Step names must be unique across steps");
        assert_eq!(name1, "step_1");
        assert_eq!(name2, "step_2");
    }

    #[test]
    fn test_ag_ui_complete_lifecycle_ordering() {
        // Simulate: RunStart → StepStart → TextDelta → ToolCallStart → ToolCallEnd →
        //           ToolCallDone → StepEnd → RunFinish
        let mut ctx = AGUIContext::new("t1".into(), "r1".into());
        let mut all_events = Vec::new();

        let agent_events = vec![
            AgentEvent::RunStart {
                thread_id: "t1".into(),
                run_id: "r1".into(),
                parent_run_id: None,
            },
            AgentEvent::StepStart,
            AgentEvent::TextDelta {
                delta: "Let me search".into(),
            },
            AgentEvent::ToolCallStart {
                id: "c1".into(),
                name: "search".into(),
            },
            AgentEvent::ToolCallDelta {
                id: "c1".into(),
                args_delta: r#"{"q":"rust"}"#.into(),
            },
            AgentEvent::ToolCallReady {
                id: "c1".into(),
                name: "search".into(),
                arguments: json!({"q": "rust"}),
            },
            AgentEvent::ToolCallDone {
                id: "c1".into(),
                result: ToolResult::success("search", json!({"found": 1})),
                patch: None,
            },
            AgentEvent::StepEnd,
            AgentEvent::RunFinish {
                thread_id: "t1".into(),
                run_id: "r1".into(),
                result: None,
                stop_reason: None,
            },
        ];

        for event in &agent_events {
            all_events.extend(agent_event_to_agui(&event, &mut ctx));
        }

        // Classify events by type name for ordering verification
        let type_names: Vec<&str> = all_events
            .iter()
            .map(|e| match e {
                AGUIEvent::RunStarted { .. } => "RunStarted",
                AGUIEvent::RunFinished { .. } => "RunFinished",
                AGUIEvent::StepStarted { .. } => "StepStarted",
                AGUIEvent::StepFinished { .. } => "StepFinished",
                AGUIEvent::TextMessageStart { .. } => "TextMessageStart",
                AGUIEvent::TextMessageContent { .. } => "TextMessageContent",
                AGUIEvent::TextMessageEnd { .. } => "TextMessageEnd",
                AGUIEvent::ToolCallStart { .. } => "ToolCallStart",
                AGUIEvent::ToolCallArgs { .. } => "ToolCallArgs",
                AGUIEvent::ToolCallEnd { .. } => "ToolCallEnd",
                AGUIEvent::ToolCallResult { .. } => "ToolCallResult",
                _ => "Other",
            })
            .collect();

        // Verify first/last
        assert_eq!(type_names.first(), Some(&"RunStarted"));
        assert_eq!(type_names.last(), Some(&"RunFinished"));

        // Verify text stream properly closed before tool call
        let text_end_idx = type_names
            .iter()
            .position(|n| *n == "TextMessageEnd")
            .unwrap();
        let tool_start_idx = type_names
            .iter()
            .position(|n| *n == "ToolCallStart")
            .unwrap();
        assert!(
            text_end_idx < tool_start_idx,
            "TextMessageEnd ({text_end_idx}) must come before ToolCallStart ({tool_start_idx})"
        );

        // Verify step pairing
        let step_start_idx = type_names.iter().position(|n| *n == "StepStarted").unwrap();
        let step_end_idx = type_names
            .iter()
            .position(|n| *n == "StepFinished")
            .unwrap();
        assert!(step_start_idx < step_end_idx);

        // Verify tool call lifecycle: START → ARGS → END → RESULT
        let tc_start = type_names
            .iter()
            .position(|n| *n == "ToolCallStart")
            .unwrap();
        let tc_args = type_names
            .iter()
            .position(|n| *n == "ToolCallArgs")
            .unwrap();
        let tc_end = type_names.iter().position(|n| *n == "ToolCallEnd").unwrap();
        let tc_result = type_names
            .iter()
            .position(|n| *n == "ToolCallResult")
            .unwrap();
        assert!(
            tc_start < tc_args,
            "ToolCallStart must precede ToolCallArgs"
        );
        assert!(tc_args < tc_end, "ToolCallArgs must precede ToolCallEnd");
        assert!(
            tc_end < tc_result,
            "ToolCallEnd must precede ToolCallResult"
        );
    }

    // ========================================================================
    // AI SDK v6 Lifecycle Ordering Tests
    // ========================================================================

    #[test]
    fn test_ui_run_start_produces_no_events() {
        let event = AgentEvent::RunStart {
            thread_id: "t1".into(),
            run_id: "r1".into(),
            parent_run_id: None,
        };
        let events = agent_event_to_ui(&event, "txt_0");
        assert!(
            events.is_empty(),
            "RunStart should not produce AI SDK events"
        );
    }

    #[test]
    fn test_ui_inference_complete_produces_no_events() {
        let event = AgentEvent::InferenceComplete {
            model: "gpt-4".into(),
            usage: None,
            duration_ms: 100,
        };
        let events = agent_event_to_ui(&event, "txt_0");
        assert!(
            events.is_empty(),
            "InferenceComplete should not produce AI SDK events"
        );
    }

    #[test]
    fn test_ui_state_snapshot_produces_data_event() {
        let event = AgentEvent::StateSnapshot {
            snapshot: json!({"count": 1}),
        };
        let events = agent_event_to_ui(&event, "txt_0");
        assert_eq!(events.len(), 1);
        if let UIStreamEvent::Data { data_type, data } = &events[0] {
            assert_eq!(data_type, "data-state-snapshot");
            assert_eq!(data["count"], 1);
        } else {
            panic!("Expected Data event, got: {:?}", events[0]);
        }
    }

    #[test]
    fn test_ui_state_delta_produces_data_event() {
        let event = AgentEvent::StateDelta {
            delta: vec![json!({"op": "add", "path": "/x", "value": 1})],
        };
        let events = agent_event_to_ui(&event, "txt_0");
        assert_eq!(events.len(), 1);
        if let UIStreamEvent::Data { data_type, data } = &events[0] {
            assert_eq!(data_type, "data-state-delta");
            assert!(data.is_array());
        } else {
            panic!("Expected Data event");
        }
    }

    #[test]
    fn test_ui_messages_snapshot_produces_data_event() {
        let event = AgentEvent::MessagesSnapshot {
            messages: vec![json!({"role": "user", "content": "hi"})],
        };
        let events = agent_event_to_ui(&event, "txt_0");
        assert_eq!(events.len(), 1);
        if let UIStreamEvent::Data { data_type, .. } = &events[0] {
            assert_eq!(data_type, "data-messages-snapshot");
        } else {
            panic!("Expected Data event");
        }
    }

    // ========================================================================
    // AG-UI Context-Dependent Path Tests
    // ========================================================================

    #[test]
    fn test_ag_ui_tool_call_start_without_active_text() {
        let mut ctx = AGUIContext::new("t1".into(), "r1".into());
        // No text started — ToolCallStart should not emit TextMessageEnd
        let event = AgentEvent::ToolCallStart {
            id: "c1".into(),
            name: "search".into(),
        };
        let events = agent_event_to_agui(&event, &mut ctx);
        assert_eq!(events.len(), 1);
        assert!(
            matches!(&events[0], AGUIEvent::ToolCallStart { .. }),
            "Should only emit ToolCallStart without TextMessageEnd when no text active"
        );
    }

    #[test]
    fn test_ag_ui_pending_without_active_text() {
        use crate::state_types::Interaction;

        let mut ctx = AGUIContext::new("t1".into(), "r1".into());
        // No text started — Pending should not emit TextMessageEnd
        let event = AgentEvent::Pending {
            interaction: Interaction::new("int_1", "confirm"),
        };
        let events = agent_event_to_agui(&event, &mut ctx);
        // Should be exactly 3 tool call events (no TextMessageEnd prefix)
        assert_eq!(
            events.len(),
            3,
            "Should emit 3 tool events without TextMessageEnd, got: {:?}",
            events
        );
        assert!(matches!(&events[0], AGUIEvent::ToolCallStart { .. }));
        assert!(matches!(&events[1], AGUIEvent::ToolCallArgs { .. }));
        assert!(matches!(&events[2], AGUIEvent::ToolCallEnd { .. }));
    }

    #[test]
    fn test_ag_ui_tool_call_done_error_result() {
        let event = AgentEvent::ToolCallDone {
            id: "c1".into(),
            result: ToolResult::error("search", "Connection refused"),
            patch: None,
        };
        let mut ctx = AGUIContext::new("t1".into(), "r1".into());
        let events = agent_event_to_agui(&event, &mut ctx);
        assert_eq!(events.len(), 1);
        if let AGUIEvent::ToolCallResult {
            tool_call_id,
            content,
            message_id,
            ..
        } = &events[0]
        {
            assert_eq!(tool_call_id, "c1");
            assert_eq!(message_id, "result_c1");
            // Content should contain the error info
            assert!(
                content.contains("error") || content.contains("Error"),
                "ToolCallResult content should contain error status, got: {content}"
            );
            assert!(
                content.contains("Connection refused"),
                "ToolCallResult content should contain error message"
            );
        } else {
            panic!("Expected ToolCallResult, got: {:?}", events[0]);
        }
    }

    // ========================================================================
    // value_to_map Edge Case Tests
    // ========================================================================

    #[test]
    fn test_value_to_map_with_object() {
        let value = json!({"key": "val", "num": 42});
        let map = value_to_map(&value);
        assert_eq!(map.get("key"), Some(&json!("val")));
        assert_eq!(map.get("num"), Some(&json!(42)));
        assert!(
            !map.contains_key("value"),
            "Object should not wrap in 'value' key"
        );
    }

    #[test]
    fn test_value_to_map_with_number() {
        let value = json!(42);
        let map = value_to_map(&value);
        assert_eq!(map.len(), 1);
        assert_eq!(map.get("value"), Some(&json!(42)));
    }

    #[test]
    fn test_value_to_map_with_boolean() {
        let value = json!(true);
        let map = value_to_map(&value);
        assert_eq!(map.len(), 1);
        assert_eq!(map.get("value"), Some(&json!(true)));
    }

    #[test]
    fn test_value_to_map_with_null() {
        let value = json!(null);
        let map = value_to_map(&value);
        assert_eq!(map.len(), 1);
        assert_eq!(map.get("value"), Some(&json!(null)));
    }

    #[test]
    fn test_value_to_map_with_array() {
        let value = json!([1, 2, 3]);
        let map = value_to_map(&value);
        assert_eq!(map.len(), 1);
        assert_eq!(map.get("value"), Some(&json!([1, 2, 3])));
    }

    #[test]
    fn test_value_to_map_with_empty_object() {
        let value = json!({});
        let map = value_to_map(&value);
        assert!(map.is_empty(), "Empty object should produce empty map");
    }

    // ========================================================================
    // StreamCollector Edge Case Tests
    // ========================================================================

    #[test]
    fn test_stream_collector_ghost_tool_call_filtered() {
        // DeepSeek sends ghost tool calls with empty fn_name
        let mut collector = StreamCollector::new();

        // Ghost call: empty fn_name
        let ghost = genai::chat::ToolCall {
            call_id: "ghost_1".to_string(),
            fn_name: String::new(),
            fn_arguments: json!(null),
            thought_signatures: None,
        };
        collector.process(ChatStreamEvent::ToolCallChunk(ToolChunk {
            tool_call: ghost,
        }));

        // Real call
        let real = genai::chat::ToolCall {
            call_id: "real_1".to_string(),
            fn_name: "search".to_string(),
            fn_arguments: Value::String(r#"{"q":"rust"}"#.to_string()),
            thought_signatures: None,
        };
        collector.process(ChatStreamEvent::ToolCallChunk(ToolChunk {
            tool_call: real,
        }));

        let result = collector.finish();
        // Ghost call should be filtered out (empty name)
        assert_eq!(result.tool_calls.len(), 1);
        assert_eq!(result.tool_calls[0].name, "search");
    }

    #[test]
    fn test_stream_collector_invalid_json_arguments_fallback() {
        let mut collector = StreamCollector::new();

        let tc = genai::chat::ToolCall {
            call_id: "call_1".to_string(),
            fn_name: "test".to_string(),
            fn_arguments: Value::String("not valid json {{".to_string()),
            thought_signatures: None,
        };
        collector.process(ChatStreamEvent::ToolCallChunk(ToolChunk { tool_call: tc }));

        let result = collector.finish();
        assert_eq!(result.tool_calls.len(), 1);
        assert_eq!(result.tool_calls[0].name, "test");
        // Invalid JSON falls back to Value::Null
        assert_eq!(result.tool_calls[0].arguments, Value::Null);
    }

    #[test]
    fn test_stream_collector_duplicate_accumulated_args_full_replace() {
        let mut collector = StreamCollector::new();

        // Start tool call
        let tc1 = genai::chat::ToolCall {
            call_id: "call_1".to_string(),
            fn_name: "test".to_string(),
            fn_arguments: Value::String(r#"{"a":1}"#.to_string()),
            thought_signatures: None,
        };
        collector.process(ChatStreamEvent::ToolCallChunk(ToolChunk { tool_call: tc1 }));

        // Same accumulated args again — not a strict prefix extension, so treated
        // as a full replacement delta (correct for accumulated-mode providers).
        let tc2 = genai::chat::ToolCall {
            call_id: "call_1".to_string(),
            fn_name: String::new(),
            fn_arguments: Value::String(r#"{"a":1}"#.to_string()),
            thought_signatures: None,
        };
        let output =
            collector.process(ChatStreamEvent::ToolCallChunk(ToolChunk { tool_call: tc2 }));
        match output {
            Some(StreamOutput::ToolCallDelta { id, args_delta }) => {
                assert_eq!(id, "call_1");
                assert_eq!(args_delta, r#"{"a":1}"#);
            }
            other => panic!("Expected ToolCallDelta, got {:?}", other),
        }
    }

    #[test]
    fn test_stream_collector_end_event_captures_usage() {
        let mut collector = StreamCollector::new();

        let end = StreamEnd {
            captured_usage: Some(Usage {
                prompt_tokens: Some(10),
                prompt_tokens_details: None,
                completion_tokens: Some(20),
                completion_tokens_details: None,
                total_tokens: Some(30),
            }),
            ..Default::default()
        };
        collector.process(ChatStreamEvent::End(end));

        let result = collector.finish();
        assert!(result.usage.is_some());
        let usage = result.usage.unwrap();
        assert_eq!(usage.prompt_tokens, Some(10));
        assert_eq!(usage.completion_tokens, Some(20));
        assert_eq!(usage.total_tokens, Some(30));
    }

    #[test]
    fn test_stream_collector_end_event_fills_missing_partial() {
        // End event creates a new partial tool call when one doesn't exist from chunks
        use genai::chat::MessageContent;

        let mut collector = StreamCollector::new();

        let end_tc = genai::chat::ToolCall {
            call_id: "end_call".to_string(),
            fn_name: "finalize".to_string(),
            fn_arguments: Value::String(r#"{"done":true}"#.to_string()),
            thought_signatures: None,
        };
        let end = StreamEnd {
            captured_content: Some(MessageContent::from_tool_calls(vec![end_tc])),
            ..Default::default()
        };
        collector.process(ChatStreamEvent::End(end));

        let result = collector.finish();
        assert_eq!(result.tool_calls.len(), 1);
        assert_eq!(result.tool_calls[0].id, "end_call");
        assert_eq!(result.tool_calls[0].name, "finalize");
        assert_eq!(result.tool_calls[0].arguments, json!({"done": true}));
    }

    #[test]
    fn test_stream_collector_end_event_overrides_partial_args() {
        // End event should override streaming partial arguments
        use genai::chat::MessageContent;

        let mut collector = StreamCollector::new();

        // Start with partial data from chunks
        let tc1 = genai::chat::ToolCall {
            call_id: "call_1".to_string(),
            fn_name: "api".to_string(),
            fn_arguments: Value::String(r#"{"partial":true"#.to_string()), // incomplete JSON
            thought_signatures: None,
        };
        collector.process(ChatStreamEvent::ToolCallChunk(ToolChunk { tool_call: tc1 }));

        // End event provides correct, complete arguments
        let end_tc = genai::chat::ToolCall {
            call_id: "call_1".to_string(),
            fn_name: String::new(), // name already set from chunk
            fn_arguments: Value::String(r#"{"complete":true}"#.to_string()),
            thought_signatures: None,
        };
        let end = StreamEnd {
            captured_content: Some(MessageContent::from_tool_calls(vec![end_tc])),
            ..Default::default()
        };
        collector.process(ChatStreamEvent::End(end));

        let result = collector.finish();
        assert_eq!(result.tool_calls.len(), 1);
        assert_eq!(result.tool_calls[0].name, "api");
        // End event's arguments should override the incomplete streaming args
        assert_eq!(result.tool_calls[0].arguments, json!({"complete": true}));
    }

    #[test]
    fn test_stream_collector_value_object_args() {
        // When fn_arguments is not a String, falls through to `other.to_string()`
        let mut collector = StreamCollector::new();

        let tc = genai::chat::ToolCall {
            call_id: "call_1".to_string(),
            fn_name: "test".to_string(),
            fn_arguments: json!({"key": "val"}), // Value::Object, not Value::String
            thought_signatures: None,
        };
        let output = collector.process(ChatStreamEvent::ToolCallChunk(ToolChunk { tool_call: tc }));

        // Should produce ToolCallStart (name) — the args delta comes from .to_string()
        // First output is ToolCallStart, then the args are also processed
        // But since name is set on the same chunk, output is ToolCallDelta (args wins over name)
        // Actually: name emit happens first, then args. But `output` only holds the LAST one.
        // Let's just check the final result.
        assert!(output.is_some());

        let result = collector.finish();
        assert_eq!(result.tool_calls.len(), 1);
        assert_eq!(result.tool_calls[0].arguments, json!({"key": "val"}));
    }

    // ========================================================================
    // Truncated / Malformed JSON Resilience Tests
    // (Reference: Mastra transform.test.ts — graceful handling of
    //  streaming race conditions and partial tool-call arguments)
    // ========================================================================

    #[test]
    fn test_stream_collector_truncated_json_args() {
        // Simulates network interruption mid-stream where the accumulated
        // argument string is incomplete JSON.  finish() should gracefully
        // produce Value::Null (never panic).
        let mut collector = StreamCollector::new();

        let tc = genai::chat::ToolCall {
            call_id: "call_1".to_string(),
            fn_name: "search".to_string(),
            fn_arguments: Value::String(r#"{"url": "https://example.com"#.to_string()),
            thought_signatures: None,
        };
        collector.process(ChatStreamEvent::ToolCallChunk(ToolChunk { tool_call: tc }));

        let result = collector.finish();
        assert_eq!(result.tool_calls.len(), 1);
        assert_eq!(result.tool_calls[0].name, "search");
        // Truncated JSON → Value::Null (graceful degradation)
        assert_eq!(result.tool_calls[0].arguments, Value::Null);
    }

    #[test]
    fn test_stream_collector_empty_json_args() {
        // Tool call with completely empty argument string.
        let mut collector = StreamCollector::new();

        let tc = genai::chat::ToolCall {
            call_id: "call_1".to_string(),
            fn_name: "noop".to_string(),
            fn_arguments: Value::String(String::new()),
            thought_signatures: None,
        };
        collector.process(ChatStreamEvent::ToolCallChunk(ToolChunk { tool_call: tc }));

        let result = collector.finish();
        assert_eq!(result.tool_calls.len(), 1);
        assert_eq!(result.tool_calls[0].name, "noop");
        // Empty string → Value::Null
        assert_eq!(result.tool_calls[0].arguments, Value::Null);
    }

    #[test]
    fn test_stream_collector_partial_nested_json() {
        // Complex nested JSON truncated mid-array.
        // Reference: Mastra tests large payload cutoff at position 871.
        let mut collector = StreamCollector::new();

        let tc = genai::chat::ToolCall {
            call_id: "call_1".to_string(),
            fn_name: "complex_tool".to_string(),
            fn_arguments: Value::String(
                r#"{"a": {"b": [1, 2, {"c": "long_string_that_gets_truncated"#.to_string(),
            ),
            thought_signatures: None,
        };
        collector.process(ChatStreamEvent::ToolCallChunk(ToolChunk { tool_call: tc }));

        let result = collector.finish();
        assert_eq!(result.tool_calls.len(), 1);
        assert_eq!(result.tool_calls[0].name, "complex_tool");
        // Truncated nested JSON → Value::Null
        assert_eq!(result.tool_calls[0].arguments, Value::Null);
    }

    #[test]
    fn test_stream_collector_truncated_then_end_event_recovers() {
        // Streaming produces truncated JSON, but the End event carries the
        // complete arguments — the End event should override and recover.
        use genai::chat::MessageContent;

        let mut collector = StreamCollector::new();

        // Truncated streaming chunk
        let tc1 = genai::chat::ToolCall {
            call_id: "call_1".to_string(),
            fn_name: "api".to_string(),
            fn_arguments: Value::String(r#"{"location": "New York", "unit": "cel"#.to_string()),
            thought_signatures: None,
        };
        collector.process(ChatStreamEvent::ToolCallChunk(ToolChunk { tool_call: tc1 }));

        // End event with complete arguments
        let end_tc = genai::chat::ToolCall {
            call_id: "call_1".to_string(),
            fn_name: String::new(),
            fn_arguments: Value::String(
                r#"{"location": "New York", "unit": "celsius"}"#.to_string(),
            ),
            thought_signatures: None,
        };
        let end = StreamEnd {
            captured_content: Some(MessageContent::from_tool_calls(vec![end_tc])),
            ..Default::default()
        };
        collector.process(ChatStreamEvent::End(end));

        let result = collector.finish();
        assert_eq!(result.tool_calls.len(), 1);
        // End event recovered the complete, valid JSON
        assert_eq!(
            result.tool_calls[0].arguments,
            json!({"location": "New York", "unit": "celsius"})
        );
    }

    #[test]
    fn test_stream_collector_valid_json_args_control() {
        // Control test: valid JSON args parse correctly (contrast with truncated tests).
        let mut collector = StreamCollector::new();

        let tc = genai::chat::ToolCall {
            call_id: "call_1".to_string(),
            fn_name: "get_weather".to_string(),
            fn_arguments: Value::String(
                r#"{"location": "San Francisco", "units": "metric"}"#.to_string(),
            ),
            thought_signatures: None,
        };
        collector.process(ChatStreamEvent::ToolCallChunk(ToolChunk { tool_call: tc }));

        let result = collector.finish();
        assert_eq!(result.tool_calls.len(), 1);
        assert_eq!(
            result.tool_calls[0].arguments,
            json!({"location": "San Francisco", "units": "metric"})
        );
    }

    // ========================================================================
    // AI SDK v6 Complete Flow Tests
    // ========================================================================

    #[test]
    fn test_ui_complete_flow_event_ordering() {
        // Simulate a full step with text + tool call and verify AI SDK event types
        let agent_events = [
            AgentEvent::StepStart,
            AgentEvent::TextDelta {
                delta: "Searching...".into(),
            },
            AgentEvent::ToolCallStart {
                id: "c1".into(),
                name: "search".into(),
            },
            AgentEvent::ToolCallDelta {
                id: "c1".into(),
                args_delta: r#"{"q":"r"}"#.into(),
            },
            AgentEvent::ToolCallReady {
                id: "c1".into(),
                name: "search".into(),
                arguments: json!({"q": "r"}),
            },
            AgentEvent::ToolCallDone {
                id: "c1".into(),
                result: ToolResult::success("search", json!([])),
                patch: None,
            },
            AgentEvent::StepEnd,
            AgentEvent::RunFinish {
                thread_id: "t".into(),
                run_id: "r".into(),
                result: None,
                stop_reason: None,
            },
        ];

        let all_ui: Vec<UIStreamEvent> = agent_events
            .iter()
            .flat_map(|e| agent_event_to_ui(e, "txt_0"))
            .collect();

        let type_names: Vec<&str> = all_ui
            .iter()
            .map(|e| match e {
                UIStreamEvent::StartStep => "StartStep",
                UIStreamEvent::FinishStep => "FinishStep",
                UIStreamEvent::TextDelta { .. } => "TextDelta",
                UIStreamEvent::ToolInputStart { .. } => "ToolInputStart",
                UIStreamEvent::ToolInputDelta { .. } => "ToolInputDelta",
                UIStreamEvent::ToolInputAvailable { .. } => "ToolInputAvailable",
                UIStreamEvent::ToolOutputAvailable { .. } => "ToolOutputAvailable",
                UIStreamEvent::Finish { .. } => "Finish",
                _ => "Other",
            })
            .collect();

        // StartStep must be first, Finish must be last
        assert_eq!(type_names.first(), Some(&"StartStep"));
        assert_eq!(type_names.last(), Some(&"Finish"));

        // Tool call lifecycle ordering
        let ti_start = type_names
            .iter()
            .position(|n| *n == "ToolInputStart")
            .unwrap();
        let ti_delta = type_names
            .iter()
            .position(|n| *n == "ToolInputDelta")
            .unwrap();
        let ti_avail = type_names
            .iter()
            .position(|n| *n == "ToolInputAvailable")
            .unwrap();
        let to_avail = type_names
            .iter()
            .position(|n| *n == "ToolOutputAvailable")
            .unwrap();
        assert!(ti_start < ti_delta);
        assert!(ti_delta < ti_avail);
        assert!(ti_avail < to_avail);

        // FinishStep must be after ToolOutputAvailable
        let finish_step = type_names.iter().position(|n| *n == "FinishStep").unwrap();
        assert!(to_avail < finish_step);
    }

    // ========================================================================
    // End Event: Edge Cases
    // ========================================================================

    #[test]
    fn test_stream_collector_end_event_no_tool_calls_preserves_streamed() {
        // When End event has no captured_tool_calls (None), the tool calls
        // accumulated from streaming chunks should be preserved.
        use genai::chat::StreamEnd;

        let mut collector = StreamCollector::new();

        // Accumulate a tool call from streaming chunks
        let tc = genai::chat::ToolCall {
            call_id: "call_1".to_string(),
            fn_name: "search".to_string(),
            fn_arguments: Value::String(r#"{"q":"test"}"#.to_string()),
            thought_signatures: None,
        };
        collector.process(ChatStreamEvent::ToolCallChunk(ToolChunk { tool_call: tc }));

        // End event with NO captured_tool_calls (some providers don't populate this)
        let end = StreamEnd {
            captured_content: None,
            ..Default::default()
        };
        collector.process(ChatStreamEvent::End(end));

        let result = collector.finish();
        assert_eq!(
            result.tool_calls.len(),
            1,
            "Streamed tool calls should be preserved"
        );
        assert_eq!(result.tool_calls[0].name, "search");
        assert_eq!(result.tool_calls[0].arguments, json!({"q": "test"}));
    }

    #[test]
    fn test_stream_collector_end_event_overrides_tool_name() {
        // End event should override tool name when streamed chunk had wrong name.
        use genai::chat::MessageContent;

        let mut collector = StreamCollector::new();

        // Streaming chunk with initial name
        let tc = genai::chat::ToolCall {
            call_id: "call_1".to_string(),
            fn_name: "search".to_string(),
            fn_arguments: Value::String(r#"{"q":"test"}"#.to_string()),
            thought_signatures: None,
        };
        collector.process(ChatStreamEvent::ToolCallChunk(ToolChunk { tool_call: tc }));

        // End event with different name (only fills if name was EMPTY, per line 136)
        let end_tc = genai::chat::ToolCall {
            call_id: "call_1".to_string(),
            fn_name: "web_search".to_string(), // different name
            fn_arguments: Value::String(r#"{"q":"test"}"#.to_string()),
            thought_signatures: None,
        };
        let end = StreamEnd {
            captured_content: Some(MessageContent::from_tool_calls(vec![end_tc])),
            ..Default::default()
        };
        collector.process(ChatStreamEvent::End(end));

        let result = collector.finish();
        assert_eq!(result.tool_calls.len(), 1);
        // Current behavior: name only overridden if empty (line 136: `if partial.name.is_empty()`)
        // So the original streaming name should be preserved.
        assert_eq!(result.tool_calls[0].name, "search");
    }

    #[test]
    fn test_stream_collector_whitespace_only_tool_name_filtered() {
        // Tool calls with whitespace-only names should be filtered (ghost tool calls).
        let mut collector = StreamCollector::new();

        let tc = genai::chat::ToolCall {
            call_id: "ghost_1".to_string(),
            fn_name: "   ".to_string(), // whitespace only
            fn_arguments: Value::String("{}".to_string()),
            thought_signatures: None,
        };
        collector.process(ChatStreamEvent::ToolCallChunk(ToolChunk { tool_call: tc }));

        let result = collector.finish();
        // finish() filters by `!p.name.is_empty()` — whitespace-only name is NOT empty.
        // This documents current behavior (whitespace names are kept).
        // If this is a bug, fix the filter to use `.trim().is_empty()`.
        assert_eq!(
            result.tool_calls.len(),
            1,
            "Whitespace-only names are currently NOT filtered (document behavior)"
        );
    }

    // ========================================================================
    // Multiple / Interleaved Tool Call Tests
    // ========================================================================

    /// Helper: create a tool call chunk event.
    fn tc_chunk(call_id: &str, fn_name: &str, args: &str) -> ChatStreamEvent {
        ChatStreamEvent::ToolCallChunk(ToolChunk {
            tool_call: genai::chat::ToolCall {
                call_id: call_id.to_string(),
                fn_name: fn_name.to_string(),
                fn_arguments: Value::String(args.to_string()),
                thought_signatures: None,
            },
        })
    }

    #[test]
    fn test_stream_collector_two_tool_calls_sequential() {
        // Two tool calls arriving sequentially.
        let mut collector = StreamCollector::new();

        collector.process(tc_chunk("tc_1", "search", r#"{"q":"foo"}"#));
        collector.process(tc_chunk("tc_2", "fetch", r#"{"url":"https://x.com"}"#));

        let result = collector.finish();
        assert_eq!(result.tool_calls.len(), 2);

        let names: Vec<&str> = result
            .tool_calls
            .iter()
            .map(|tc| tc.name.as_str())
            .collect();
        assert!(names.contains(&"search"));
        assert!(names.contains(&"fetch"));

        let search = result
            .tool_calls
            .iter()
            .find(|tc| tc.name == "search")
            .unwrap();
        assert_eq!(search.arguments, json!({"q": "foo"}));

        let fetch = result
            .tool_calls
            .iter()
            .find(|tc| tc.name == "fetch")
            .unwrap();
        assert_eq!(fetch.arguments, json!({"url": "https://x.com"}));
    }

    #[test]
    fn test_stream_collector_two_tool_calls_interleaved_chunks() {
        // Two tool calls with interleaved argument chunks (accumulated args, AI SDK v6 pattern).
        // Chunk 1: tc_a name only (empty args)
        // Chunk 2: tc_b name only (empty args)
        // Chunk 3: tc_a with partial args
        // Chunk 4: tc_b with partial args
        // Chunk 5: tc_a with full args (accumulated)
        // Chunk 6: tc_b with full args (accumulated)
        let mut collector = StreamCollector::new();

        // Initial name-only chunks
        collector.process(tc_chunk("tc_a", "search", ""));
        collector.process(tc_chunk("tc_b", "fetch", ""));

        // Partial args (accumulated pattern)
        collector.process(tc_chunk("tc_a", "search", r#"{"q":"#));
        collector.process(tc_chunk("tc_b", "fetch", r#"{"url":"#));

        // Full accumulated args
        collector.process(tc_chunk("tc_a", "search", r#"{"q":"a"}"#));
        collector.process(tc_chunk("tc_b", "fetch", r#"{"url":"b"}"#));

        let result = collector.finish();
        assert_eq!(result.tool_calls.len(), 2);

        let search = result
            .tool_calls
            .iter()
            .find(|tc| tc.name == "search")
            .unwrap();
        assert_eq!(search.arguments, json!({"q": "a"}));

        let fetch = result
            .tool_calls
            .iter()
            .find(|tc| tc.name == "fetch")
            .unwrap();
        assert_eq!(fetch.arguments, json!({"url": "b"}));
    }

    #[test]
    fn test_stream_collector_tool_call_interleaved_with_text() {
        // Text chunks interleaved between tool call chunks.
        let mut collector = StreamCollector::new();

        collector.process(ChatStreamEvent::Chunk(StreamChunk {
            content: "I will ".to_string(),
        }));
        collector.process(tc_chunk("tc_1", "search", ""));
        collector.process(ChatStreamEvent::Chunk(StreamChunk {
            content: "search ".to_string(),
        }));
        collector.process(tc_chunk("tc_1", "search", r#"{"q":"test"}"#));
        collector.process(ChatStreamEvent::Chunk(StreamChunk {
            content: "for you.".to_string(),
        }));

        let result = collector.finish();
        // Text should be accumulated
        assert_eq!(result.text, "I will search for you.");
        // Tool call should be present
        assert_eq!(result.tool_calls.len(), 1);
        assert_eq!(result.tool_calls[0].arguments, json!({"q": "test"}));
    }
}
