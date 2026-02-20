#![allow(missing_docs)]

use tirea_contract::{AgentEvent, Interaction, ProtocolInputAdapter, Role, TerminationReason};
use tirea_protocol_ag_ui::{
    AGUIContext, AGUIEvent, AGUIMessage, AgUiInputAdapter, RunAgentRequest,
};

#[test]
fn agui_context_pending_closes_text_and_emits_interaction_tool_events() {
    let mut ctx = AGUIContext::new("thread_1".to_string(), "run_1".to_string());

    let text_events = ctx.on_agent_event(&AgentEvent::TextDelta {
        delta: "hello".to_string(),
    });
    assert!(matches!(text_events[0], AGUIEvent::TextMessageStart { .. }));
    assert!(matches!(
        text_events[1],
        AGUIEvent::TextMessageContent { .. }
    ));

    let pending_events = ctx.on_agent_event(&AgentEvent::Pending {
        interaction: Interaction::new("int_1", "confirm")
            .with_message("Allow this action?")
            .with_parameters(serde_json::json!({ "approved": true })),
    });

    // Pending closes text stream but no longer emits tool call events
    assert_eq!(pending_events.len(), 1);
    assert!(matches!(
        pending_events[0],
        AGUIEvent::TextMessageEnd { .. }
    ));
}

#[test]
fn agui_context_interaction_requested_emits_tool_call_events_for_new_ids() {
    let mut ctx = AGUIContext::new("thread_1".to_string(), "run_1".to_string());

    // InteractionRequested with a never-seen ID → should emit TOOL_CALL_START/ARGS/END
    let events = ctx.on_agent_event(&AgentEvent::InteractionRequested {
        interaction: Interaction::new("new_call_1", "tool:PermissionConfirm")
            .with_message("Allow this?")
            .with_parameters(serde_json::json!({ "question": "ok?" })),
    });

    assert_eq!(events.len(), 3);
    assert!(matches!(events[0], AGUIEvent::ToolCallStart { .. }));
    assert!(matches!(events[1], AGUIEvent::ToolCallArgs { .. }));
    assert!(matches!(events[2], AGUIEvent::ToolCallEnd { .. }));
}

#[test]
fn agui_context_interaction_requested_skips_already_emitted_tool_call_ids() {
    let mut ctx = AGUIContext::new("thread_1".to_string(), "run_1".to_string());

    // First: LLM streams a ToolCallStart for "call_copy"
    let _ = ctx.on_agent_event(&AgentEvent::ToolCallStart {
        id: "call_copy".to_string(),
        name: "copyToClipboard".to_string(),
    });

    // Then: InteractionRequested for same ID → should be suppressed (already emitted)
    let events = ctx.on_agent_event(&AgentEvent::InteractionRequested {
        interaction: Interaction::new("call_copy", "tool:copyToClipboard")
            .with_parameters(serde_json::json!({ "text": "hello" })),
    });

    assert!(events.is_empty());
}

#[test]
fn agui_context_interaction_requested_closes_open_text_stream() {
    let mut ctx = AGUIContext::new("thread_1".to_string(), "run_1".to_string());

    // Start text
    let _ = ctx.on_agent_event(&AgentEvent::TextDelta {
        delta: "thinking...".to_string(),
    });
    assert!(ctx.is_text_open());

    // InteractionRequested for new ID with open text → closes text first
    let events = ctx.on_agent_event(&AgentEvent::InteractionRequested {
        interaction: Interaction::new("new_int_1", "tool:PermissionConfirm")
            .with_parameters(serde_json::json!({})),
    });

    // TEXT_MESSAGE_END + TOOL_CALL_START + TOOL_CALL_ARGS + TOOL_CALL_END
    assert_eq!(events.len(), 4);
    assert!(matches!(events[0], AGUIEvent::TextMessageEnd { .. }));
    assert!(matches!(events[1], AGUIEvent::ToolCallStart { .. }));
    assert!(!ctx.is_text_open());
}

#[test]
fn agui_input_adapter_converts_messages() {
    let mut request = RunAgentRequest::new("thread_1", "run_1").with_messages(vec![
        AGUIMessage::assistant("skip this"),
        AGUIMessage::user("hello"),
        AGUIMessage::tool(r#"{"approved":true}"#, "int_1"),
    ]);
    request.parent_run_id = Some("parent_1".to_string());

    let run = AgUiInputAdapter::to_run_request("agent_1".to_string(), request);

    assert_eq!(run.agent_id, "agent_1");
    assert_eq!(run.thread_id.as_deref(), Some("thread_1"));
    assert_eq!(run.run_id.as_deref(), Some("run_1"));
    assert_eq!(run.parent_run_id.as_deref(), Some("parent_1"));
    assert_eq!(run.messages.len(), 2);
    assert_eq!(run.messages[0].role, Role::User);
    assert_eq!(run.messages[1].role, Role::Tool);
}

#[test]
fn agui_context_cancelled_run_finish_maps_to_run_error() {
    let mut ctx = AGUIContext::new("thread_1".to_string(), "run_1".to_string());

    let events = ctx.on_agent_event(&AgentEvent::RunFinish {
        thread_id: "thread_1".to_string(),
        run_id: "run_1".to_string(),
        result: None,
        termination: TerminationReason::Cancelled,
    });

    assert_eq!(events.len(), 1);
    match &events[0] {
        AGUIEvent::RunError { code, .. } => {
            assert_eq!(code.as_deref(), Some("CANCELLED"));
        }
        other => panic!("expected RUN_ERROR for cancelled run, got: {other:?}"),
    }
}

#[test]
fn agui_context_non_cancelled_run_finish_maps_to_run_finished() {
    let mut ctx = AGUIContext::new("thread_1".to_string(), "run_1".to_string());

    let events = ctx.on_agent_event(&AgentEvent::RunFinish {
        thread_id: "thread_1".to_string(),
        run_id: "run_1".to_string(),
        result: Some(serde_json::json!({"response":"ok"})),
        termination: TerminationReason::NaturalEnd,
    });

    assert_eq!(events.len(), 1);
    assert!(matches!(events[0], AGUIEvent::RunFinished { .. }));
}
