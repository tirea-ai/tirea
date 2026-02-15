use carve_agent_runtime_contract::AgentEvent;
use carve_protocol_ai_sdk_v6::AiSdkEncoder;

#[test]
fn encoder_adopts_first_step_start_message_id() {
    let step_msg_id = "pre-gen-assistant-uuid".to_string();
    let mut encoder = AiSdkEncoder::new("run_12345678".to_string());

    assert_eq!(encoder.message_id(), "msg_run_1234");

    let _ = encoder.on_agent_event(&AgentEvent::StepStart {
        message_id: step_msg_id.clone(),
    });
    assert_eq!(
        encoder.message_id(),
        step_msg_id,
        "AiSdkEncoder must adopt StepStart.message_id"
    );

    let _ = encoder.on_agent_event(&AgentEvent::StepStart {
        message_id: "second-step-id".to_string(),
    });
    assert_eq!(
        encoder.message_id(),
        "pre-gen-assistant-uuid",
        "AiSdkEncoder must keep the first StepStart.message_id"
    );
}
