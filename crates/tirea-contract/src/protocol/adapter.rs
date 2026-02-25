//! Shared protocol adapter traits.

use serde::Serialize;

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
