use super::{AgUiEventContext, Event};
use tirea_contract::{AgentEvent, ProtocolOutputEncoder};

pub struct AgUiProtocolEncoder {
    ctx: AgUiEventContext,
}

impl AgUiProtocolEncoder {
    pub fn new() -> Self {
        Self {
            ctx: AgUiEventContext::new(),
        }
    }
}

impl Default for AgUiProtocolEncoder {
    fn default() -> Self {
        Self::new()
    }
}

impl ProtocolOutputEncoder for AgUiProtocolEncoder {
    type InputEvent = AgentEvent;
    type Event = Event;

    fn on_agent_event(&mut self, ev: &AgentEvent) -> Vec<Self::Event> {
        self.ctx.on_agent_event(ev)
    }
}
