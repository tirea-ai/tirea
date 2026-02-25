use super::{AgUiEventContext, Event};
use tirea_contract::{AgentEvent, Transcoder};

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

impl Transcoder for AgUiProtocolEncoder {
    type Input = AgentEvent;
    type Output = Event;

    fn transcode(&mut self, item: &AgentEvent) -> Vec<Event> {
        self.ctx.on_agent_event(item)
    }
}
