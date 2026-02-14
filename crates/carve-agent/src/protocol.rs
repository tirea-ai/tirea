use crate::ag_ui::{AGUIContext, AGUIEvent, AGUIMessage, MessageRole, RunAgentRequest};
use crate::ui_stream::{AiSdkEncoder, UIStreamEvent};
use crate::{agent_event_to_agui, AgentEvent, Message, Role, Visibility};
use serde::{Deserialize, Serialize};
use serde_json::json;

/// Protocol input boundary:
/// protocol request -> internal `RunRequest`.
pub trait ProtocolInputAdapter {
    type Request;

    fn to_run_request(agent_id: String, request: Self::Request) -> crate::agent_os::RunRequest;
}

/// Protocol output boundary:
/// internal `AgentEvent` -> protocol event(s).
///
/// Transport layers (SSE/NATS/etc.) should depend on this trait and remain
/// agnostic to protocol-specific branching.
pub trait ProtocolOutputEncoder {
    type Event: Serialize;

    fn prologue(&mut self) -> Vec<Self::Event> {
        Vec::new()
    }

    fn on_agent_event(&mut self, ev: &AgentEvent) -> Vec<Self::Event>;

    fn epilogue(&mut self) -> Vec<Self::Event> {
        Vec::new()
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct AiSdkRunRequest {
    #[serde(rename = "sessionId")]
    pub thread_id: String,
    pub input: String,
    #[serde(rename = "runId")]
    pub run_id: Option<String>,
}

pub struct AiSdkInputAdapter;

impl ProtocolInputAdapter for AiSdkInputAdapter {
    type Request = AiSdkRunRequest;

    fn to_run_request(agent_id: String, request: Self::Request) -> crate::agent_os::RunRequest {
        crate::agent_os::RunRequest {
            agent_id,
            thread_id: if request.thread_id.trim().is_empty() {
                None
            } else {
                Some(request.thread_id)
            },
            run_id: request.run_id,
            resource_id: None,
            initial_state: None,
            messages: vec![Message::user(request.input)],
            runtime: std::collections::HashMap::new(),
        }
    }
}

pub struct AgUiInputAdapter;

impl ProtocolInputAdapter for AgUiInputAdapter {
    type Request = RunAgentRequest;

    fn to_run_request(agent_id: String, request: Self::Request) -> crate::agent_os::RunRequest {
        let mut runtime = std::collections::HashMap::new();
        if let Some(parent_run_id) = request.parent_run_id.clone() {
            runtime.insert(
                "parent_run_id".to_string(),
                serde_json::Value::String(parent_run_id),
            );
        }

        crate::agent_os::RunRequest {
            agent_id,
            thread_id: Some(request.thread_id),
            run_id: Some(request.run_id),
            resource_id: None,
            initial_state: request.state,
            messages: convert_agui_messages(&request.messages),
            runtime,
        }
    }
}

fn convert_agui_messages(messages: &[AGUIMessage]) -> Vec<Message> {
    messages
        .iter()
        .filter(|m| m.role != MessageRole::Assistant)
        .map(|m| {
            let role = match m.role {
                MessageRole::System | MessageRole::Developer => Role::System,
                MessageRole::User => Role::User,
                MessageRole::Assistant => Role::Assistant,
                MessageRole::Tool => Role::Tool,
            };
            Message {
                id: m.id.clone(),
                role,
                content: m.content.clone(),
                tool_calls: None,
                tool_call_id: m.tool_call_id.clone(),
                visibility: Visibility::default(),
                metadata: None,
            }
        })
        .collect()
}

#[derive(Debug)]
struct AgUiEncoderState {
    ctx: AGUIContext,
    emitted_run_finished: bool,
    stopped: bool,
}

impl AgUiEncoderState {
    fn new(thread_id: String, run_id: String) -> Self {
        Self {
            ctx: AGUIContext::new(thread_id, run_id),
            emitted_run_finished: false,
            stopped: false,
        }
    }

    fn on_agent_event(&mut self, ev: &AgentEvent) -> Vec<AGUIEvent> {
        // After an error (RUN_ERROR), suppress all further events including
        // RunFinish — the RUN_ERROR is already the terminal event and sending
        // RUN_FINISHED after it violates the AG-UI protocol.
        if self.stopped {
            return Vec::new();
        }

        match ev {
            AgentEvent::Error { .. } => {
                self.stopped = true;
                self.emitted_run_finished = true; // RUN_ERROR is terminal
            }
            AgentEvent::RunFinish { .. } => {
                self.emitted_run_finished = true;
            }
            // Skip Pending events: their interaction-to-tool-call conversion
            // is redundant in AG-UI — the LLM's own TOOL_CALL events already
            // inform the client about frontend tool calls.
            // However, we must still close any open text stream.
            AgentEvent::Pending { .. } => {
                let mut events = Vec::new();
                if self.ctx.end_text() {
                    events.push(AGUIEvent::text_message_end(&self.ctx.message_id));
                }
                return events;
            }
            _ => {}
        }

        agent_event_to_agui(ev, &mut self.ctx)
    }

    fn fallback_finished_current(&mut self) -> Vec<AGUIEvent> {
        if self.stopped || self.emitted_run_finished {
            return Vec::new();
        }
        let mut events = Vec::new();
        if self.ctx.end_text() {
            events.push(AGUIEvent::text_message_end(&self.ctx.message_id));
        }
        events.push(AGUIEvent::run_finished(
            &self.ctx.thread_id,
            &self.ctx.run_id,
            None,
        ));
        events
    }
}

pub struct AiSdkProtocolEncoder {
    inner: AiSdkEncoder,
    run_info: Option<UIStreamEvent>,
}

impl AiSdkProtocolEncoder {
    pub fn new(run_id: String, thread_id: Option<String>) -> Self {
        let run_info = thread_id.map(|thread_id| {
            UIStreamEvent::data(
                "run-info",
                json!({
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

impl ProtocolOutputEncoder for AiSdkProtocolEncoder {
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

pub struct AgUiProtocolEncoder {
    inner: AgUiEncoderState,
}

impl AgUiProtocolEncoder {
    pub fn new(thread_id: String, run_id: String) -> Self {
        Self {
            inner: AgUiEncoderState::new(thread_id, run_id),
        }
    }
}

impl ProtocolOutputEncoder for AgUiProtocolEncoder {
    type Event = AGUIEvent;

    fn on_agent_event(&mut self, ev: &AgentEvent) -> Vec<Self::Event> {
        self.inner.on_agent_event(ev)
    }

    fn epilogue(&mut self) -> Vec<Self::Event> {
        self.inner.fallback_finished_current()
    }
}
