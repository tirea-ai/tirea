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
    let event = UIStreamEvent::tool_input_available("call_1", "search", json!({"query": "rust"}));
    let json = serde_json::to_string(&event).unwrap();
    assert!(json.contains(r#""type":"tool-input-available""#));
    assert!(json.contains(r#""toolCallId":"call_1""#));
    assert!(json.contains(r#""toolName":"search""#));
    assert!(json.contains(r#""input""#));
}

#[test]
fn test_tool_output_available_serialization() {
    let event = UIStreamEvent::tool_output_available("call_1", json!({"result": "found 3 items"}));
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
        vec![
            "start",
            "text-start",
            "text-delta",
            "text-delta",
            "text-end",
            "finish"
        ]
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
fn test_encoder_step_end_closes_text() {
    use crate::stream::AgentEvent;

    let mut enc = AiSdkEncoder::new("run_1".to_string());

    // Open text
    enc.on_agent_event(&AgentEvent::TextDelta {
        delta: "hi".to_string(),
    });

    // StepEnd should close text before finish-step
    let out = enc.on_agent_event(&AgentEvent::StepEnd);
    assert_eq!(out.len(), 2);
    assert!(matches!(out[0], UIStreamEvent::TextEnd { .. }));
    assert!(matches!(out[1], UIStreamEvent::FinishStep));
}

#[test]
fn test_encoder_step_end_without_text_no_text_end() {
    use crate::stream::AgentEvent;

    let mut enc = AiSdkEncoder::new("run_1".to_string());

    // StepEnd without text should not emit text-end
    let out = enc.on_agent_event(&AgentEvent::StepEnd);
    assert_eq!(out.len(), 1);
    assert!(matches!(out[0], UIStreamEvent::FinishStep));
}

#[test]
fn test_encoder_text_tool_step_end_full_lifecycle() {
    // Reproduces the AI SDK v6 "text-end for missing text part" bug:
    // text-start → text-delta → finish-step(resets activeTextParts) → text-end → crash
    // After fix: text-end is emitted BEFORE finish-step.
    use crate::stream::AgentEvent;

    let mut enc = AiSdkEncoder::new("run_1".to_string());
    let mut all_events = enc.prologue();

    // Step 1: text + tool + step end
    for ev in &[
        AgentEvent::StepStart {
            message_id: String::new(),
        },
        AgentEvent::TextDelta {
            delta: "Plan: ".to_string(),
        },
        AgentEvent::ToolCallStart {
            id: "tc1".to_string(),
            name: "search".to_string(),
        },
        AgentEvent::ToolCallDone {
            id: "tc1".to_string(),
            result: crate::ToolResult::success("search", serde_json::json!([])),
            patch: None,
            message_id: String::new(),
        },
        AgentEvent::StepEnd,
    ] {
        all_events.extend(enc.on_agent_event(ev));
    }

    // Step 2: text + finish
    for ev in &[
        AgentEvent::StepStart {
            message_id: String::new(),
        },
        AgentEvent::TextDelta {
            delta: "Done".to_string(),
        },
        AgentEvent::StepEnd,
        AgentEvent::RunFinish {
            thread_id: "t".to_string(),
            run_id: "run_1".to_string(),
            result: None,
            stop_reason: None,
        },
    ] {
        all_events.extend(enc.on_agent_event(ev));
    }

    let types: Vec<String> = all_events
        .iter()
        .map(|e| {
            let json = serde_json::to_string(e).unwrap();
            let v: serde_json::Value = serde_json::from_str(&json).unwrap();
            v["type"].as_str().unwrap().to_string()
        })
        .collect();

    // Every text-start must have a text-end BEFORE finish-step
    assert_eq!(
        types,
        vec![
            "start",
            "start-step",
            "text-start",
            "text-delta", // txt_0
            "text-end",   // txt_0 closed before tool
            "tool-input-start",
            "tool-output-available",
            "finish-step", // safe: no open text
            "start-step",
            "text-start",
            "text-delta", // txt_1
            "text-end",   // txt_1 closed before finish-step
            "finish-step",
            "finish", // no extra text-end needed
        ]
    );
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
