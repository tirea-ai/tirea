use super::{AGUIContext, AGUIEvent};
use crate::protocol::ProtocolOutputEncoder;
use crate::AgentEvent;

pub struct AgUiProtocolEncoder {
    ctx: AGUIContext,
}

impl AgUiProtocolEncoder {
    pub fn new(thread_id: String, run_id: String) -> Self {
        Self {
            ctx: AGUIContext::new(thread_id, run_id),
        }
    }
}

impl ProtocolOutputEncoder for AgUiProtocolEncoder {
    type Event = AGUIEvent;

    fn on_agent_event(&mut self, ev: &AgentEvent) -> Vec<Self::Event> {
        self.ctx.on_agent_event(ev)
    }
}
