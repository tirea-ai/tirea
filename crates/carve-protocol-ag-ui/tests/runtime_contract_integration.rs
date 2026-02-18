#![allow(missing_docs)]

use carve_agent_contract::{AgentEvent, Interaction, ProtocolInputAdapter, Role, TerminationReason};
use carve_protocol_ag_ui::{
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
