//! Shared protocol adapter traits.

use crate::{Message, RunRequest};
use serde::Serialize;

/// Protocol input boundary: protocol request -> internal runtime `RunRequest`.
pub trait ProtocolInputAdapter {
    /// Protocol-specific request payload type.
    type Request;

    /// Convert protocol request into runtime request.
    fn to_run_request(agent_id: String, request: Self::Request) -> RunRequest;
}

/// Protocol history boundary: stored message -> protocol history message.
pub trait ProtocolHistoryEncoder {
    /// Protocol-specific history message type.
    type HistoryMessage: Serialize;

    /// Encode one internal message.
    fn encode_message(msg: &Message) -> Self::HistoryMessage;

    /// Encode multiple internal messages.
    fn encode_messages<'a>(
        msgs: impl IntoIterator<Item = &'a Message>,
    ) -> Vec<Self::HistoryMessage> {
        msgs.into_iter().map(Self::encode_message).collect()
    }
}

/// Protocol output boundary: internal runtime event -> protocol event(s).
pub trait ProtocolOutputEncoder {
    /// Runtime event type consumed by this encoder.
    type InputEvent;
    /// Protocol-specific output event type.
    type Event: Serialize;

    /// Optional prologue events emitted before runtime stream starts.
    fn prologue(&mut self) -> Vec<Self::Event> {
        Vec::new()
    }

    /// Map one runtime event to zero or more protocol events.
    fn on_agent_event(&mut self, ev: &Self::InputEvent) -> Vec<Self::Event>;

    /// Optional epilogue events emitted after runtime stream ends.
    fn epilogue(&mut self) -> Vec<Self::Event> {
        Vec::new()
    }
}
