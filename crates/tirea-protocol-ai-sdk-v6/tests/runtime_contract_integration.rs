#![allow(missing_docs)]

use tirea_contract::{AgentEvent, StoppedReason, TerminationReason, Transcoder};
use tirea_protocol_ai_sdk_v6::{AiSdkEncoder, UIStreamEvent};

#[test]
fn run_start_emits_message_start_and_run_info() {
    let mut encoder = AiSdkEncoder::new();

    let events = encoder.transcode(&AgentEvent::RunStart {
        thread_id: "thread_1".into(),
        run_id: "run_12345678".into(),
        parent_run_id: None,
    });
    assert_eq!(events.len(), 2);
    assert!(matches!(events[0], UIStreamEvent::MessageStart { .. }));
    assert!(matches!(
        events[1],
        UIStreamEvent::Data { ref data_type, .. } if data_type == "data-run-info"
    ));
}

#[test]
fn protocol_encoder_closes_text_before_tool_and_maps_finish_reason() {
    let mut encoder = AiSdkEncoder::new();

    let text_events = encoder.transcode(&AgentEvent::TextDelta {
        delta: "hello".to_string(),
    });
    assert_eq!(text_events.len(), 2);
    assert!(matches!(text_events[0], UIStreamEvent::TextStart { .. }));
    assert!(matches!(text_events[1], UIStreamEvent::TextDelta { .. }));

    let tool_start_events = encoder.transcode(&AgentEvent::ToolCallStart {
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

    let finish_events = encoder.transcode(&AgentEvent::RunFinish {
        thread_id: "thread_1".to_string(),
        run_id: "run_1".to_string(),
        result: None,
        termination: TerminationReason::Stopped(StoppedReason::new("max_rounds_reached")),
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

#[test]
fn protocol_encoder_maps_cancelled_run_to_abort() {
    let mut encoder = AiSdkEncoder::new();

    let events = encoder.transcode(&AgentEvent::RunFinish {
        thread_id: "thread_1".to_string(),
        run_id: "run_cancel".to_string(),
        result: None,
        termination: TerminationReason::Cancelled,
    });
    assert_eq!(events.len(), 1);
    assert!(matches!(events[0], UIStreamEvent::Abort { .. }));
}
