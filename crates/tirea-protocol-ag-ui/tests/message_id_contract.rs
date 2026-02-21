#![allow(missing_docs)]

use serde_json::json;
use tirea_contract::{AgentEvent, ToolResult};
use tirea_protocol_ag_ui::{AgUiEventContext, Event};

#[test]
fn text_message_start_uses_step_start_message_id() {
    let step_msg_id = "pre-gen-assistant-uuid".to_string();
    let mut ctx = AgUiEventContext::new("thread1".to_string(), "run1".to_string());

    let step_events = ctx.on_agent_event(&AgentEvent::StepStart {
        message_id: step_msg_id.clone(),
    });
    assert!(!step_events.is_empty());

    let text_events = ctx.on_agent_event(&AgentEvent::TextDelta {
        delta: "Hello".to_string(),
    });

    let text_start = text_events
        .iter()
        .find(|e| matches!(e, Event::TextMessageStart { .. }))
        .expect("AG-UI should emit TextMessageStart on first TextDelta");

    if let Event::TextMessageStart { message_id, .. } = text_start {
        assert_eq!(
            message_id, &step_msg_id,
            "AG-UI TextMessageStart.message_id must equal StepStart.message_id"
        );
    }
}

#[test]
fn tool_call_result_uses_tool_call_done_message_id() {
    let tool_msg_id = "pre-gen-tool-uuid".to_string();
    let mut ctx = AgUiEventContext::new("thread1".to_string(), "run1".to_string());

    let result_events = ctx.on_agent_event(&AgentEvent::ToolCallDone {
        id: "call_1".to_string(),
        result: ToolResult::success("echo", json!({"echoed": "test"})),
        patch: None,
        message_id: tool_msg_id.clone(),
    });

    let tool_result = result_events
        .iter()
        .find(|e| matches!(e, Event::ToolCallResult { .. }))
        .expect("AG-UI should emit ToolCallResult");

    if let Event::ToolCallResult { message_id, .. } = tool_result {
        assert_eq!(
            message_id, &tool_msg_id,
            "AG-UI ToolCallResult.message_id must equal ToolCallDone.message_id"
        );
    }
}
