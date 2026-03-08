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

    pub fn new_with_frontend_run_id(run_id: impl Into<String>) -> Self {
        Self {
            ctx: AgUiEventContext::new().with_frontend_run_id(run_id),
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
