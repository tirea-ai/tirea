use carve_agent_runtime_contract::{AgentEvent, StopReason};
use carve_protocol_ai_sdk_v6::{AiSdkV6ProtocolEncoder, UIStreamEvent};
use carve_protocol_contract::ProtocolOutputEncoder;

#[test]
fn protocol_encoder_prologue_includes_run_info_event() {
    let mut encoder =
        AiSdkV6ProtocolEncoder::new("run_1".to_string(), Some("thread_1".to_string()));

    let events = encoder.prologue();
    assert_eq!(events.len(), 2);
    assert!(matches!(events[0], UIStreamEvent::MessageStart { .. }));
    assert!(matches!(
        events[1],
        UIStreamEvent::Data { ref data_type, .. } if data_type == "data-run-info"
    ));
}

#[test]
fn protocol_encoder_closes_text_before_tool_and_maps_finish_reason() {
    let mut encoder = AiSdkV6ProtocolEncoder::new("run_1".to_string(), None);

    let text_events = encoder.on_agent_event(&AgentEvent::TextDelta {
        delta: "hello".to_string(),
    });
    assert_eq!(text_events.len(), 2);
    assert!(matches!(text_events[0], UIStreamEvent::TextStart { .. }));
    assert!(matches!(text_events[1], UIStreamEvent::TextDelta { .. }));

    let tool_start_events = encoder.on_agent_event(&AgentEvent::ToolCallStart {
        id: "call_1".to_string(),
        name: "search".to_string(),
    });
    assert_eq!(tool_start_events.len(), 2);
    assert!(matches!(
        tool_start_events[0],
        UIStreamEvent::TextEnd { .. }
    ));
    assert!(matches!(
        tool_start_events[1],
        UIStreamEvent::ToolInputStart { .. }
    ));

    let finish_events = encoder.on_agent_event(&AgentEvent::RunFinish {
        thread_id: "thread_1".to_string(),
        run_id: "run_1".to_string(),
        result: None,
        stop_reason: Some(StopReason::MaxRoundsReached),
    });
    assert_eq!(finish_events.len(), 1);
    assert!(matches!(
        finish_events[0],
        UIStreamEvent::Finish {
            finish_reason: Some(ref reason),
            ..
        } if reason == "length"
    ));
}
