use super::{AgUiEventContext, Event};
use tirea_contract::{AgentEvent, ProtocolOutputEncoder};

pub struct AgUiProtocolEncoder {
    ctx: AgUiEventContext,
}

impl AgUiProtocolEncoder {
    pub fn new(thread_id: String, run_id: String) -> Self {
        Self {
            ctx: AgUiEventContext::new(thread_id, run_id),
        }
    }
}

impl ProtocolOutputEncoder for AgUiProtocolEncoder {
    type InputEvent = AgentEvent;
    type Event = Event;

    fn on_agent_event(&mut self, ev: &AgentEvent) -> Vec<Self::Event> {
        self.ctx.on_agent_event(ev)
    }
}
