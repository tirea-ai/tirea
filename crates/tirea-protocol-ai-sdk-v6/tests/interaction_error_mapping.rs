//! Integration tests for AI SDK interaction error event mapping.

use serde_json::json;
use tirea_contract::AgentEvent;
use tirea_protocol_ai_sdk_v6::{AiSdkEncoder, UIStreamEvent};

#[test]
fn interaction_resolved_error_maps_to_tool_output_error_event() {
    let mut enc = AiSdkEncoder::new("run_err".into());
    let events = enc.on_agent_event(&AgentEvent::InteractionResolved {
        interaction_id: "ask_call_2".to_string(),
        result: json!({ "approved": false, "error": "frontend validation failed" }),
    });

    assert!(
        events.iter().any(|ev| matches!(
            ev,
            UIStreamEvent::ToolOutputError {
                tool_call_id,
                error_text,
                ..
            }
            if tool_call_id == "ask_call_2" && error_text == "frontend validation failed"
        )),
        "errored interaction should emit tool-output-error"
    );
}
