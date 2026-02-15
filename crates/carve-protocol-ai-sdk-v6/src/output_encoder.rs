use super::{AiSdkEncoder, UIStreamEvent, AI_SDK_VERSION};
use carve_agent_contract::AgentEvent;
use carve_protocol_contract::ProtocolOutputEncoder;
use serde_json::json;

const RUN_INFO_EVENT_NAME: &str = "run-info";

pub struct AiSdkV6ProtocolEncoder {
    inner: AiSdkEncoder,
    run_info: Option<UIStreamEvent>,
}

impl AiSdkV6ProtocolEncoder {
    pub fn new(run_id: String, thread_id: Option<String>) -> Self {
        let run_info = thread_id.map(|thread_id| {
            UIStreamEvent::data(
                RUN_INFO_EVENT_NAME,
                json!({
                    "protocol": "ai-sdk-ui-message-stream",
                    "protocolVersion": "v1",
                    "aiSdkVersion": AI_SDK_VERSION,
                    "threadId": thread_id,
                    "runId": run_id
                }),
            )
        });
        Self {
            inner: AiSdkEncoder::new(run_id),
            run_info,
        }
    }
}

impl ProtocolOutputEncoder for AiSdkV6ProtocolEncoder {
    type InputEvent = AgentEvent;
    type Event = UIStreamEvent;

    fn prologue(&mut self) -> Vec<Self::Event> {
        let mut events = self.inner.prologue();
        if let Some(run_info) = self.run_info.take() {
            events.push(run_info);
        }
        events
    }

    fn on_agent_event(&mut self, ev: &AgentEvent) -> Vec<Self::Event> {
        self.inner.on_agent_event(ev)
    }
}
