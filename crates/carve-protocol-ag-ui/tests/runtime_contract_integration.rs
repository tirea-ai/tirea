use carve_agent_runtime_contract::{
    AgentEvent, Interaction, RUNTIME_INTERACTION_FRONTEND_TOOLS_KEY,
    RUNTIME_INTERACTION_RESPONSES_KEY,
};
use carve_protocol_ag_ui::{
    AGUIContext, AGUIEvent, AGUIMessage, AGUIToolDef, AgUiInputAdapter, RunAgentRequest,
};
use carve_protocol_contract::ProtocolInputAdapter;
use carve_thread_model::Role;

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

    assert_eq!(pending_events.len(), 4);
    assert!(matches!(
        pending_events[0],
        AGUIEvent::TextMessageEnd { .. }
    ));
    assert!(matches!(pending_events[1], AGUIEvent::ToolCallStart { .. }));
    assert!(matches!(pending_events[2], AGUIEvent::ToolCallArgs { .. }));
    assert!(matches!(pending_events[3], AGUIEvent::ToolCallEnd { .. }));
}

#[test]
fn agui_input_adapter_converts_messages_and_runtime_values() {
    let mut request = RunAgentRequest::new("thread_1", "run_1").with_messages(vec![
        AGUIMessage::assistant("skip this"),
        AGUIMessage::user("hello"),
        AGUIMessage::tool(r#"{"approved":true}"#, "int_1"),
    ]);
    request.parent_run_id = Some("parent_1".to_string());
    request.tools.push(AGUIToolDef::frontend(
        "ask_user",
        "frontend interaction tool",
    ));

    let run = AgUiInputAdapter::to_run_request("agent_1".to_string(), request);

    assert_eq!(run.agent_id, "agent_1");
    assert_eq!(run.thread_id.as_deref(), Some("thread_1"));
    assert_eq!(run.run_id.as_deref(), Some("run_1"));
    assert_eq!(run.messages.len(), 2);
    assert_eq!(run.messages[0].role, Role::User);
    assert_eq!(run.messages[1].role, Role::Tool);

    let frontend_tools = run
        .runtime
        .get(RUNTIME_INTERACTION_FRONTEND_TOOLS_KEY)
        .and_then(|value| value.as_array())
        .expect("frontend tools runtime key should exist");
    assert_eq!(frontend_tools.len(), 1);
    assert_eq!(frontend_tools[0].as_str(), Some("ask_user"));

    let responses = run
        .runtime
        .get(RUNTIME_INTERACTION_RESPONSES_KEY)
        .and_then(|value| value.as_array())
        .expect("interaction responses runtime key should exist");
    assert_eq!(responses.len(), 1);
    assert_eq!(responses[0]["interaction_id"].as_str(), Some("int_1"));
    assert_eq!(responses[0]["result"]["approved"].as_bool(), Some(true));

    assert_eq!(
        run.runtime
            .get("parent_run_id")
            .and_then(|value| value.as_str()),
        Some("parent_1")
    );
}
