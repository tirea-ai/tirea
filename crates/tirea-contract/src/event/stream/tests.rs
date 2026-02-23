use super::AgentEvent;

#[test]
fn reasoning_delta_roundtrip() {
    let event = AgentEvent::ReasoningDelta {
        delta: "think".to_string(),
    };
    let raw = serde_json::to_string(&event).expect("serialize reasoning delta");
    let restored: AgentEvent = serde_json::from_str(&raw).expect("deserialize reasoning delta");
    assert!(matches!(restored, AgentEvent::ReasoningDelta { delta } if delta == "think"));
}

#[test]
fn reasoning_encrypted_value_roundtrip() {
    let event = AgentEvent::ReasoningEncryptedValue {
        encrypted_value: "opaque".to_string(),
    };
    let raw = serde_json::to_string(&event).expect("serialize reasoning encrypted value");
    let restored: AgentEvent =
        serde_json::from_str(&raw).expect("deserialize reasoning encrypted value");
    assert!(matches!(
        restored,
        AgentEvent::ReasoningEncryptedValue { encrypted_value } if encrypted_value == "opaque"
    ));
}
