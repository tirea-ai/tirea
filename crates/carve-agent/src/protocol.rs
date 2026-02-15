use crate::{AgentEvent, Message};
use serde::Serialize;

pub use crate::ag_ui::{AgUiInputAdapter, AgUiProtocolEncoder};
pub use crate::ai_sdk_v6::{
    AiSdkV6InputAdapter, AiSdkV6ProtocolEncoder, AiSdkV6RunRequest, AI_SDK_VERSION,
};

/// Protocol input boundary:
/// protocol request -> internal `RunRequest`.
pub trait ProtocolInputAdapter {
    type Request;

    fn to_run_request(agent_id: String, request: Self::Request) -> crate::agent_os::RunRequest;
}

/// Protocol history boundary:
/// stored `Message` → protocol-specific history message format.
///
/// This is the reverse of [`ProtocolInputAdapter`]: while input adapters
/// convert protocol messages into internal `Message`s, this trait converts
/// stored internal `Message`s back into protocol-native formats for REST
/// query endpoints (e.g. loading chat history on page refresh).
pub trait ProtocolHistoryEncoder {
    type HistoryMessage: Serialize;

    fn encode_message(msg: &Message) -> Self::HistoryMessage;

    fn encode_messages<'a>(
        msgs: impl IntoIterator<Item = &'a Message>,
    ) -> Vec<Self::HistoryMessage> {
        msgs.into_iter().map(Self::encode_message).collect()
    }
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
