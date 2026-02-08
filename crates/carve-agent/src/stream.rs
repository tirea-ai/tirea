//! Streaming response handling for LLM responses.
//!
//! This module provides three event systems:
//! - `AgentEvent`: The internal event type for the agent loop
//! - `UIStreamEvent`: AI SDK v6 compatible events for frontend integration
//! - `AGUIEvent`: AG-UI protocol compatible events for CopilotKit integration
//!
//! Use `AgentEvent::to_ui_events()` to convert to AI SDK format.
//! Use `AgentEvent::to_ag_ui_events()` to convert to AG-UI format.

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
    current_tool_id: Option<String>,
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

                // Get or create partial tool call
                let partial =
                    self.tool_calls
                        .entry(call_id.clone())
                        .or_insert_with(|| PartialToolCall {
                            id: call_id.clone(),
                            name: String::new(),
                            arguments: String::new(),
                        });

                // Update name if provided (non-empty)
                if !tool_chunk.tool_call.fn_name.is_empty() && partial.name.is_empty() {
                    partial.name = tool_chunk.tool_call.fn_name.clone();
                    self.current_tool_id = Some(call_id.clone());
                    return Some(StreamOutput::ToolCallStart {
                        id: call_id,
                        name: partial.name.clone(),
                    });
                }

                // Accumulate arguments
                let args_str = tool_chunk.tool_call.fn_arguments.to_string();
                if args_str != "null" && !args_str.is_empty() {
                    partial.arguments.push_str(&args_str);
                    return Some(StreamOutput::ToolCallDelta {
                        id: call_id,
                        args_delta: args_str,
                    });
                }

                None
            }
            ChatStreamEvent::End(end) => {
                // Capture any tool calls from the end event
                if let Some(tool_calls) = end.captured_tool_calls() {
                    for tc in tool_calls {
                        self.tool_calls.insert(
                            tc.call_id.clone(),
                            PartialToolCall {
                                id: tc.call_id.clone(),
                                name: tc.fn_name.clone(),
                                arguments: tc.fn_arguments.to_string(),
                            },
                        );
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
        let tool_calls: Vec<ToolCall> = self
            .tool_calls
            .into_values()
            .map(|p| {
                let arguments = serde_json::from_str(&p.arguments).unwrap_or(Value::Null);
                ToolCall::new(p.id, p.name, arguments)
            })
            .collect();

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
/// These events represent the internal agent loop state.
/// - For AI SDK v6 compatible events, use `to_ui_events()` to convert to `UIStreamEvent`.
/// - For AG-UI compatible events, use `to_ag_ui_events()` to convert to `AGUIEvent`.
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
    /// Agent loop completed with final response.
    Done { response: String },
    /// Stream aborted by user or system.
    Aborted { reason: String },
    /// Error occurred.
    Error { message: String },
}

impl AgentEvent {
    /// Convert this event to AI SDK v6 compatible UI stream events.
    ///
    /// Some internal events map to multiple UI events. For example,
    /// `ToolCallDone` produces `tool-output-available`.
    pub fn to_ui_events(&self, text_id: &str) -> Vec<UIStreamEvent> {
        match self {
            // Lifecycle events - AI SDK doesn't have direct equivalents
            AgentEvent::RunStart { .. } => vec![],
            AgentEvent::RunFinish { .. } => vec![UIStreamEvent::finish()],

            // Text events
            AgentEvent::TextDelta { delta } => {
                vec![UIStreamEvent::text_delta(text_id, delta)]
            }

            // Tool events
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
            // Step events
            AgentEvent::StepStart => {
                vec![UIStreamEvent::start_step()]
            }
            AgentEvent::StepEnd => {
                vec![UIStreamEvent::finish_step()]
            }

            // State events - use custom data events for AI SDK
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

            // Activity events - use custom data events for AI SDK
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

            // Interaction events
            AgentEvent::Pending { interaction } => {
                vec![UIStreamEvent::data(
                    "interaction",
                    serde_json::to_value(interaction).unwrap_or_default(),
                )]
            }

            // Completion events
            AgentEvent::Done { .. } => {
                vec![UIStreamEvent::finish()]
            }
            AgentEvent::Aborted { reason } => {
                vec![UIStreamEvent::abort(reason)]
            }
            AgentEvent::Error { message } => {
                vec![UIStreamEvent::error(message)]
            }
            AgentEvent::InferenceComplete { .. } => vec![],
        }
    }

    /// Convert this event to AG-UI protocol compatible events.
    ///
    /// AG-UI events follow the specification at <https://docs.ag-ui.com/concepts/events>
    pub fn to_ag_ui_events(&self, ctx: &mut AGUIContext) -> Vec<AGUIEvent> {
        match self {
            // Lifecycle events
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
            } => {
                let mut events = vec![];
                // End text stream if active
                if ctx.end_text() {
                    events.push(AGUIEvent::text_message_end(&ctx.message_id));
                }
                events.push(AGUIEvent::run_finished(thread_id, run_id, result.clone()));
                events
            }

            // Text events
            AgentEvent::TextDelta { delta } => {
                let mut events = vec![];
                // Start text message if not started
                if ctx.start_text() {
                    events.push(AGUIEvent::text_message_start(&ctx.message_id));
                }
                events.push(AGUIEvent::text_message_content(&ctx.message_id, delta));
                events
            }

            // Tool events
            AgentEvent::ToolCallStart { id, name } => {
                let mut events = vec![];
                // End text stream before tool call
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
            // Step events
            AgentEvent::StepStart => {
                vec![AGUIEvent::step_started(ctx.next_step_name())]
            }
            AgentEvent::StepEnd => {
                vec![AGUIEvent::step_finished(ctx.current_step_name())]
            }

            // State events
            AgentEvent::StateSnapshot { snapshot } => {
                vec![AGUIEvent::state_snapshot(snapshot.clone())]
            }
            AgentEvent::StateDelta { delta } => {
                vec![AGUIEvent::state_delta(delta.clone())]
            }
            AgentEvent::MessagesSnapshot { messages } => {
                vec![AGUIEvent::messages_snapshot(messages.clone())]
            }

            // Activity events
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
                    replace.clone(),
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

            // Interaction events
            AgentEvent::Pending { interaction } => {
                let mut events = vec![];
                // End text stream if active
                if ctx.end_text() {
                    events.push(AGUIEvent::text_message_end(&ctx.message_id));
                }
                // Convert interaction to AG-UI tool call events
                events.extend(interaction.to_ag_ui_events());
                events
            }

            // Completion events
            AgentEvent::Done { response } => {
                let mut events = vec![];
                // End text stream if active
                if ctx.end_text() {
                    events.push(AGUIEvent::text_message_end(&ctx.message_id));
                }
                let result = if response.is_empty() {
                    None
                } else {
                    Some(serde_json::json!({ "response": response }))
                };
                events.push(AGUIEvent::run_finished(&ctx.thread_id, &ctx.run_id, result));
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

        // Test Done
        let event = AgentEvent::Done {
            response: "Final response".to_string(),
        };
        if let AgentEvent::Done { response } = event {
            assert_eq!(response, "Final response");
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
        let mut collector = StreamCollector::new();

        // Start tool call
        let tc1 = genai::chat::ToolCall {
            call_id: "call_1".to_string(),
            fn_name: "api".to_string(),
            fn_arguments: json!(null),
            thought_signatures: None,
        };
        collector.process(ChatStreamEvent::ToolCallChunk(ToolChunk { tool_call: tc1 }));

        // Accumulate arguments in chunks
        let tc2 = genai::chat::ToolCall {
            call_id: "call_1".to_string(),
            fn_name: String::new(),
            fn_arguments: json!({"url": "https://"}),
            thought_signatures: None,
        };
        collector.process(ChatStreamEvent::ToolCallChunk(ToolChunk { tool_call: tc2 }));

        let tc3 = genai::chat::ToolCall {
            call_id: "call_1".to_string(),
            fn_name: String::new(),
            fn_arguments: json!({"method": "GET"}),
            thought_signatures: None,
        };
        collector.process(ChatStreamEvent::ToolCallChunk(ToolChunk { tool_call: tc3 }));

        let result = collector.finish();
        assert_eq!(result.tool_calls.len(), 1);
        // Arguments are accumulated as strings and parsed at finish
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
        let ui_events = event.to_ui_events("txt_0");
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
        let ui_events = event.to_ui_events("txt_0");
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
        let ui_events = event.to_ui_events("txt_0");
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
        let ui_events = event.to_ui_events("txt_0");
        assert_eq!(ui_events.len(), 1);
        let json = serde_json::to_string(&ui_events[0]).unwrap();
        assert!(json.contains(r#""type":"tool-output-available""#));
        assert!(json.contains(r#""output""#));
    }

    #[test]
    fn test_agent_event_to_ui_events_step_start() {
        let event = AgentEvent::StepStart;
        let ui_events = event.to_ui_events("txt_0");
        assert_eq!(ui_events.len(), 1);
        let json = serde_json::to_string(&ui_events[0]).unwrap();
        assert!(json.contains(r#""type":"start-step""#));
    }

    #[test]
    fn test_agent_event_to_ui_events_step_end() {
        let event = AgentEvent::StepEnd;
        let ui_events = event.to_ui_events("txt_0");
        assert_eq!(ui_events.len(), 1);
        let json = serde_json::to_string(&ui_events[0]).unwrap();
        assert!(json.contains(r#""type":"finish-step""#));
    }

    #[test]
    fn test_agent_event_to_ui_events_done() {
        let event = AgentEvent::Done {
            response: "Final answer".to_string(),
        };
        let ui_events = event.to_ui_events("txt_0");
        assert_eq!(ui_events.len(), 1);
        let json = serde_json::to_string(&ui_events[0]).unwrap();
        assert!(json.contains(r#""type":"finish""#));
    }

    #[test]
    fn test_agent_event_to_ui_events_aborted() {
        let event = AgentEvent::Aborted {
            reason: "User cancelled".to_string(),
        };
        let ui_events = event.to_ui_events("txt_0");
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
        let ui_events = event.to_ui_events("txt_0");
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
        let ui_events = event.to_ui_events("txt_0");
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
        let ui_events = event.to_ui_events("txt_0");
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
        let ui_events = event.to_ui_events("txt_0");
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
        let ui_events = event.to_ui_events("txt_0");
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
        let ui_events = event.to_ui_events("txt_0");
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
        let ui_events = event.to_ui_events("txt_0");
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
        let events = event.to_ag_ui_events(&mut ctx);
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
        let events = event.to_ag_ui_events(&mut ctx);
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
            AgentEvent::Done {
                response: "Found 3 results.".to_string(),
            },
        ];

        // Convert all events to UI events
        let all_ui_events: Vec<UIStreamEvent> = events
            .iter()
            .flat_map(|e| e.to_ui_events("txt_0"))
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
            .any(|e| matches!(e, UIStreamEvent::Finish)));
    }

    // ========================================================================
    // Additional Coverage Tests
    // ========================================================================

    #[test]
    fn test_agent_event_error_conversion() {
        let event = AgentEvent::Error {
            message: "Connection timeout".to_string(),
        };
        let ui_events = event.to_ui_events("txt_0");
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
        let ui_events = event.to_ui_events("txt_0");
        assert_eq!(ui_events.len(), 1);
        assert!(
            matches!(&ui_events[0], UIStreamEvent::Abort { reason } if reason == "User cancelled")
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

        let ui_events = event.to_ui_events("txt_0");
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

        let ui_events = event.to_ui_events("txt_0");
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
        let ui_events = event.to_ui_events("txt_0");

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
        let ag_ui_events = event.to_ag_ui_events(&mut ctx);

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

        let ag_ui_events = event.to_ag_ui_events(&mut ctx);

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
        let events = event.to_ag_ui_events(&mut ctx);
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
        let events = event.to_ag_ui_events(&mut ctx);
        assert_eq!(events.len(), 1);
        assert!(matches!(events[0], AGUIEvent::ActivityDelta { .. }));
    }
}
