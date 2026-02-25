use super::{AiSdkEncoder, UIStreamEvent, AI_SDK_VERSION};
use serde_json::json;
use tirea_contract::{AgentEvent, Transcoder};

const RUN_INFO_EVENT_NAME: &str = "run-info";

pub struct AiSdkV6ProtocolEncoder {
    inner: AiSdkEncoder,
}

impl AiSdkV6ProtocolEncoder {
    pub fn new() -> Self {
        Self {
            inner: AiSdkEncoder::new(),
        }
    }
}

impl Default for AiSdkV6ProtocolEncoder {
    fn default() -> Self {
        Self::new()
    }
}

impl Transcoder for AiSdkV6ProtocolEncoder {
    type Input = AgentEvent;
    type Output = UIStreamEvent;

    fn transcode(&mut self, item: &AgentEvent) -> Vec<UIStreamEvent> {
        let mut events = self.inner.on_agent_event(item);
        if let AgentEvent::RunStart {
            thread_id, run_id, ..
        } = item
        {
            events.push(UIStreamEvent::data(
                RUN_INFO_EVENT_NAME,
                json!({
                    "protocol": "ai-sdk-ui-message-stream",
                    "protocolVersion": "v1",
                    "aiSdkVersion": AI_SDK_VERSION,
                    "threadId": thread_id,
                    "runId": run_id,
                }),
            ));
        }
        events
    }
}
