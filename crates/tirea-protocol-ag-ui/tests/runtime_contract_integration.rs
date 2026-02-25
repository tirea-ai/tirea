#![allow(missing_docs)]

use serde_json::{json, Value};
use tirea_contract::{AgentEvent, Role, TerminationReason, ToolResult};
use tirea_protocol_ag_ui::{AgUiEventContext, Event, Message, RunAgentInput};

fn make_agui_ctx(thread_id: &str, run_id: &str) -> AgUiEventContext {
    let mut ctx = AgUiEventContext::new();
    ctx.on_agent_event(&AgentEvent::RunStart {
        thread_id: thread_id.to_string(),
        run_id: run_id.to_string(),
        parent_run_id: None,
    });
    ctx
}


fn event_type(event: &Event) -> String {
    serde_json::to_value(event)
        .ok()
        .and_then(|v| v.get("type").cloned())
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_else(|| "UNKNOWN".to_string())
}

#[test]
fn agui_context_pending_closes_text_and_emits_interaction_tool_events() {
    let mut ctx = make_agui_ctx("thread_1", "run_1");

    let text_events = ctx.on_agent_event(&AgentEvent::TextDelta {
        delta: "hello".to_string(),
    });
    assert!(matches!(text_events[0], Event::TextMessageStart { .. }));
    assert!(matches!(text_events[1], Event::TextMessageContent { .. }));

    let pending_start_events = ctx.on_agent_event(&AgentEvent::ToolCallStart {
        id: "int_1".to_string(),
        name: "PermissionConfirm".to_string(),
    });
    let pending_ready_events = ctx.on_agent_event(&AgentEvent::ToolCallReady {
        id: "int_1".to_string(),
        name: "PermissionConfirm".to_string(),
        arguments: serde_json::json!({ "approved": true }),
    });

    // Pending is represented by standard tool call events.
    assert_eq!(pending_start_events.len(), 2);
    assert!(matches!(
        pending_start_events[0],
        Event::TextMessageEnd { .. }
    ));
    assert!(matches!(
        pending_start_events[1],
        Event::ToolCallStart { .. }
    ));
    assert_eq!(pending_ready_events.len(), 2);
    assert!(matches!(
        pending_ready_events[0],
        Event::ToolCallArgs { .. }
    ));
    assert!(matches!(pending_ready_events[1], Event::ToolCallEnd { .. }));
}

#[test]
fn agui_context_tool_call_ready_emits_args_and_end_when_no_deltas() {
    let mut ctx = make_agui_ctx("thread_1", "run_1");

    let start_events = ctx.on_agent_event(&AgentEvent::ToolCallStart {
        id: "new_call_1".to_string(),
        name: "PermissionConfirm".to_string(),
    });
    let ready_events = ctx.on_agent_event(&AgentEvent::ToolCallReady {
        id: "new_call_1".to_string(),
        name: "PermissionConfirm".to_string(),
        arguments: serde_json::json!({ "question": "ok?" }),
    });

    assert_eq!(start_events.len(), 1);
    assert!(matches!(start_events[0], Event::ToolCallStart { .. }));
    assert_eq!(ready_events.len(), 2);
    assert!(matches!(ready_events[0], Event::ToolCallArgs { .. }));
    assert!(matches!(ready_events[1], Event::ToolCallEnd { .. }));
}

#[test]
fn agui_context_tool_call_ready_skips_args_when_delta_already_seen() {
    let mut ctx = make_agui_ctx("thread_1", "run_1");

    let _ = ctx.on_agent_event(&AgentEvent::ToolCallStart {
        id: "call_copy".to_string(),
        name: "copyToClipboard".to_string(),
    });
    let _ = ctx.on_agent_event(&AgentEvent::ToolCallDelta {
        id: "call_copy".to_string(),
        args_delta: r#"{"text":"hello"}"#.to_string(),
    });

    let events = ctx.on_agent_event(&AgentEvent::ToolCallReady {
        id: "call_copy".to_string(),
        name: "copyToClipboard".to_string(),
        arguments: serde_json::json!({ "text": "hello" }),
    });

    assert_eq!(events.len(), 1);
    assert!(matches!(events[0], Event::ToolCallEnd { .. }));
}

#[test]
fn agui_context_tool_call_start_closes_open_text_stream() {
    let mut ctx = make_agui_ctx("thread_1", "run_1");

    // Start text
    let _ = ctx.on_agent_event(&AgentEvent::TextDelta {
        delta: "thinking...".to_string(),
    });
    assert!(ctx.is_text_open());

    // ToolCallStart for a pending frontend tool closes text first.
    let events = ctx.on_agent_event(&AgentEvent::ToolCallStart {
        id: "new_int_1".to_string(),
        name: "PermissionConfirm".to_string(),
    });

    // TEXT_MESSAGE_END + TOOL_CALL_START
    assert_eq!(events.len(), 2);
    assert!(matches!(events[0], Event::TextMessageEnd { .. }));
    assert!(matches!(events[1], Event::ToolCallStart { .. }));
    assert!(!ctx.is_text_open());
}

#[test]
fn agui_input_adapter_converts_messages() {
    let mut request = RunAgentInput::new("thread_1", "run_1").with_messages(vec![
        Message::assistant("skip this"),
        Message::user("hello"),
        Message::tool(r#"{"approved":true}"#, "int_1"),
    ]);
    request.parent_run_id = Some("parent_1".to_string());

    let run = request.into_runtime_run_request("agent_1".to_string());

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
    let mut ctx = make_agui_ctx("thread_1", "run_1");

    let events = ctx.on_agent_event(&AgentEvent::RunFinish {
        thread_id: "thread_1".to_string(),
        run_id: "run_1".to_string(),
        result: None,
        termination: TerminationReason::Cancelled,
    });

    assert_eq!(events.len(), 1);
    match &events[0] {
        Event::RunError { code, .. } => {
            assert_eq!(code.as_deref(), Some("CANCELLED"));
        }
        other => panic!("expected RUN_ERROR for cancelled run, got: {other:?}"),
    }
}

#[test]
fn agui_context_non_cancelled_run_finish_maps_to_run_finished() {
    let mut ctx = make_agui_ctx("thread_1", "run_1");

    let events = ctx.on_agent_event(&AgentEvent::RunFinish {
        thread_id: "thread_1".to_string(),
        run_id: "run_1".to_string(),
        result: Some(serde_json::json!({"response":"ok"})),
        termination: TerminationReason::NaturalEnd,
    });

    assert_eq!(events.len(), 1);
    assert!(matches!(events[0], Event::RunFinished { .. }));
}

#[test]
fn agui_contract_event_order_is_stable_for_text_tool_and_terminal_events() {
    let mut ctx = make_agui_ctx("thread_1", "run_1");
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
    let mut ctx = make_agui_ctx("thread_1", "run_1");
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
    assert!(matches!(terminal_events[0], Event::TextMessageEnd { .. }));
    assert!(matches!(terminal_events[1], Event::RunError { .. }));

    let suppressed = ctx.on_agent_event(&AgentEvent::TextDelta {
        delta: "must-not-emit".to_string(),
    });
    assert!(suppressed.is_empty());
}

#[test]
fn agui_contract_hitl_roundtrip_maps_requested_and_response_ids() {
    let mut ctx = make_agui_ctx("thread_1", "run_1");
    let requested_start_events = ctx.on_agent_event(&AgentEvent::ToolCallStart {
        id: "perm_1".to_string(),
        name: "confirm_write".to_string(),
    });
    let requested_ready_events = ctx.on_agent_event(&AgentEvent::ToolCallReady {
        id: "perm_1".to_string(),
        name: "confirm_write".to_string(),
        arguments: json!({ "path": "notes.md" }),
    });
    assert_eq!(requested_start_events.len(), 1);
    assert!(matches!(
        requested_start_events[0],
        Event::ToolCallStart { .. }
    ));
    assert_eq!(requested_ready_events.len(), 2);
    assert!(matches!(
        requested_ready_events[0],
        Event::ToolCallArgs { .. }
    ));
    assert!(matches!(
        requested_ready_events[1],
        Event::ToolCallEnd { .. }
    ));

    // ToolCallResumed is intentionally consumed by runtime and not emitted as AG-UI event.
    let resolved_events = ctx.on_agent_event(&AgentEvent::ToolCallResumed {
        target_id: "perm_1".to_string(),
        result: Value::Bool(true),
    });
    assert!(resolved_events.is_empty());

    let request = RunAgentInput::new("thread_1", "run_2").with_messages(vec![
        Message::tool("true", "perm_1"),
        Message::tool("false", "perm_2"),
    ]);
    assert_eq!(request.approved_target_ids(), vec!["perm_1"]);
    assert_eq!(request.denied_target_ids(), vec!["perm_2"]);
}

#[test]
fn agui_contract_state_snapshot_delta_is_consistent() {
    let mut ctx = make_agui_ctx("thread_1", "run_1");
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
    assert!(matches!(first_events[0], Event::StateSnapshot { .. }));

    let second_events = ctx.on_agent_event(&AgentEvent::StateSnapshot {
        snapshot: second.clone(),
    });
    assert_eq!(second_events.len(), 2);
    assert!(matches!(second_events[0], Event::StateDelta { .. }));
    assert!(matches!(second_events[1], Event::StateSnapshot { .. }));

    let delta = match &second_events[0] {
        Event::StateDelta { delta, .. } => delta.clone(),
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
