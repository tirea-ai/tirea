#![allow(missing_docs)]

use serde_json::{json, Value};
use tirea_contract::{
    AgentEvent, Interaction, ProtocolInputAdapter, Role, TerminationReason, ToolResult,
};
use tirea_protocol_ag_ui::{
    AGUIContext, AGUIEvent, AGUIMessage, AgUiInputAdapter, RunAgentRequest,
};

fn event_type(event: &AGUIEvent) -> String {
    serde_json::to_value(event)
        .ok()
        .and_then(|v| v.get("type").cloned())
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_else(|| "UNKNOWN".to_string())
}

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

#[test]
fn agui_contract_event_order_is_stable_for_text_tool_and_terminal_events() {
    let mut ctx = AGUIContext::new("thread_1".to_string(), "run_1".to_string());
    let mut stream = Vec::new();

    stream.extend(ctx.on_agent_event(&AgentEvent::RunStart {
        thread_id: "thread_1".to_string(),
        run_id: "run_1".to_string(),
        parent_run_id: None,
    }));
    stream.extend(ctx.on_agent_event(&AgentEvent::StepStart {
        message_id: "msg_step_1".to_string(),
    }));
    stream.extend(ctx.on_agent_event(&AgentEvent::TextDelta {
        delta: "thinking".to_string(),
    }));
    stream.extend(ctx.on_agent_event(&AgentEvent::ToolCallStart {
        id: "call_1".to_string(),
        name: "search".to_string(),
    }));
    stream.extend(ctx.on_agent_event(&AgentEvent::ToolCallDelta {
        id: "call_1".to_string(),
        args_delta: "{\"query\":\"ag-ui\"}".to_string(),
    }));
    stream.extend(ctx.on_agent_event(&AgentEvent::ToolCallReady {
        id: "call_1".to_string(),
        name: "search".to_string(),
        arguments: json!({ "query": "ag-ui" }),
    }));
    stream.extend(ctx.on_agent_event(&AgentEvent::ToolCallDone {
        id: "call_1".to_string(),
        result: ToolResult::success("search", json!({ "hits": 1 })),
        patch: None,
        message_id: "result_call_1".to_string(),
    }));
    stream.extend(ctx.on_agent_event(&AgentEvent::StepEnd));
    stream.extend(ctx.on_agent_event(&AgentEvent::RunFinish {
        thread_id: "thread_1".to_string(),
        run_id: "run_1".to_string(),
        result: Some(json!({ "response": "done" })),
        termination: TerminationReason::NaturalEnd,
    }));

    let types: Vec<String> = stream.iter().map(event_type).collect();
    assert_eq!(
        types,
        vec![
            "RUN_STARTED",
            "STEP_STARTED",
            "TEXT_MESSAGE_START",
            "TEXT_MESSAGE_CONTENT",
            "TEXT_MESSAGE_END",
            "TOOL_CALL_START",
            "TOOL_CALL_ARGS",
            "TOOL_CALL_END",
            "TOOL_CALL_RESULT",
            "STEP_FINISHED",
            "RUN_FINISHED",
        ]
    );
}

#[test]
fn agui_contract_terminal_event_closes_text_and_suppresses_followup_events() {
    let mut ctx = AGUIContext::new("thread_1".to_string(), "run_1".to_string());
    let _ = ctx.on_agent_event(&AgentEvent::TextDelta {
        delta: "partial".to_string(),
    });
    assert!(ctx.is_text_open());

    let terminal_events = ctx.on_agent_event(&AgentEvent::RunFinish {
        thread_id: "thread_1".to_string(),
        run_id: "run_1".to_string(),
        result: None,
        termination: TerminationReason::Cancelled,
    });
    assert_eq!(terminal_events.len(), 2);
    assert!(matches!(terminal_events[0], AGUIEvent::TextMessageEnd { .. }));
    assert!(matches!(terminal_events[1], AGUIEvent::RunError { .. }));

    let suppressed = ctx.on_agent_event(&AgentEvent::TextDelta {
        delta: "must-not-emit".to_string(),
    });
    assert!(suppressed.is_empty());
}

#[test]
fn agui_contract_hitl_roundtrip_maps_requested_and_response_ids() {
    let mut ctx = AGUIContext::new("thread_1".to_string(), "run_1".to_string());
    let interaction = Interaction::new("perm_1", "tool:confirm_write")
        .with_message("approve this write?")
        .with_parameters(json!({ "path": "notes.md" }));

    let requested_events = ctx.on_agent_event(&AgentEvent::InteractionRequested {
        interaction: interaction.clone(),
    });
    assert_eq!(requested_events.len(), 3);
    assert!(matches!(requested_events[0], AGUIEvent::ToolCallStart { .. }));
    assert!(matches!(requested_events[1], AGUIEvent::ToolCallArgs { .. }));
    assert!(matches!(requested_events[2], AGUIEvent::ToolCallEnd { .. }));

    // InteractionResolved is intentionally consumed by runtime and not emitted as AG-UI event.
    let resolved_events = ctx.on_agent_event(&AgentEvent::InteractionResolved {
        interaction_id: "perm_1".to_string(),
        result: Value::Bool(true),
    });
    assert!(resolved_events.is_empty());

    let request = RunAgentRequest::new("thread_1", "run_2").with_messages(vec![
        AGUIMessage::tool("true", "perm_1"),
        AGUIMessage::tool("false", "perm_2"),
    ]);
    assert_eq!(request.approved_interaction_ids(), vec!["perm_1"]);
    assert_eq!(request.denied_interaction_ids(), vec!["perm_2"]);
}

#[test]
fn agui_contract_state_snapshot_delta_is_consistent() {
    let mut ctx = AGUIContext::new("thread_1".to_string(), "run_1".to_string());
    let first = json!({
        "todos": ["a"],
        "meta": { "count": 1 }
    });
    let second = json!({
        "todos": ["a", "b"],
        "meta": { "count": 2 }
    });

    let first_events = ctx.on_agent_event(&AgentEvent::StateSnapshot {
        snapshot: first.clone(),
    });
    assert_eq!(first_events.len(), 1);
    assert!(matches!(first_events[0], AGUIEvent::StateSnapshot { .. }));

    let second_events = ctx.on_agent_event(&AgentEvent::StateSnapshot {
        snapshot: second.clone(),
    });
    assert_eq!(second_events.len(), 2);
    assert!(matches!(second_events[0], AGUIEvent::StateDelta { .. }));
    assert!(matches!(second_events[1], AGUIEvent::StateSnapshot { .. }));

    let delta = match &second_events[0] {
        AGUIEvent::StateDelta { delta, .. } => delta.clone(),
        _ => unreachable!(),
    };

    let mut reconstructed = first;
    let patch = json_patch::Patch(
        delta
            .iter()
            .map(|op| serde_json::from_value(op.clone()).expect("delta op must deserialize"))
            .collect(),
    );
    json_patch::patch(&mut reconstructed, &patch).expect("delta patch should apply");
    assert_eq!(reconstructed, second);
}
