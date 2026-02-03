//! Streaming response handling for LLM responses.

use crate::traits::tool::ToolResult;
use crate::types::ToolCall;
use carve_state::TrackedPatch;
use genai::chat::ChatStreamEvent;
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
                let partial = self.tool_calls.entry(call_id.clone()).or_insert_with(|| {
                    PartialToolCall {
                        id: call_id.clone(),
                        name: String::new(),
                        arguments: String::new(),
                    }
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
#[derive(Debug, Clone)]
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
}

impl StreamResult {
    /// Check if tool execution is needed.
    pub fn needs_tools(&self) -> bool {
        !self.tool_calls.is_empty()
    }
}

/// Agent loop events for streaming execution.
#[derive(Debug, Clone)]
pub enum AgentEvent {
    /// LLM text delta.
    TextDelta(String),
    /// Tool call started.
    ToolCallStart { id: String, name: String },
    /// Tool call arguments delta.
    ToolCallDelta { id: String, args_delta: String },
    /// Tool call completed.
    ToolCallDone {
        id: String,
        result: ToolResult,
        patch: Option<TrackedPatch>,
    },
    /// Turn completed (LLM finished, tools may or may not have been called).
    TurnDone {
        text: String,
        tool_calls: Vec<ToolCall>,
    },
    /// Agent loop completed with final response.
    Done { response: String },
    /// Error occurred.
    Error(String),
}

#[cfg(test)]
mod tests {
    use super::*;
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
        };
        assert!(!result.needs_tools());

        let result_with_tools = StreamResult {
            text: String::new(),
            tool_calls: vec![ToolCall::new("id", "name", serde_json::json!({}))],
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
        let event = AgentEvent::TextDelta("Hello".to_string());
        match event {
            AgentEvent::TextDelta(s) => assert_eq!(s, "Hello"),
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

        // Test TurnDone
        let event = AgentEvent::TurnDone {
            text: "Done".to_string(),
            tool_calls: vec![],
        };
        if let AgentEvent::TurnDone { text, tool_calls } = event {
            assert_eq!(text, "Done");
            assert!(tool_calls.is_empty());
        }

        // Test Done
        let event = AgentEvent::Done {
            response: "Final response".to_string(),
        };
        if let AgentEvent::Done { response } = event {
            assert_eq!(response, "Final response");
        }

        // Test Error
        let event = AgentEvent::Error("Something went wrong".to_string());
        if let AgentEvent::Error(msg) = event {
            assert!(msg.contains("wrong"));
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
            text: "This is a long response without any tool calls. It just contains text.".to_string(),
            tool_calls: vec![],
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
        let event = AgentEvent::Error("error message".to_string());
        let debug_str = format!("{:?}", event);
        assert!(debug_str.contains("Error"));
        assert!(debug_str.contains("error message"));
    }

    #[test]
    fn test_stream_result_clone() {
        let result = StreamResult {
            text: "Hello".to_string(),
            tool_calls: vec![ToolCall::new("1", "test", json!({}))],
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
}
