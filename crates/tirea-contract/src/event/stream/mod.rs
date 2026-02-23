mod envelope_meta;
mod event;
mod serde_impl;
#[cfg(test)]
mod tests;
mod wire;

pub use event::AgentEvent;

#[doc(hidden)]
pub mod internal {
    pub use super::envelope_meta::{
        clear_runtime_event_envelope_meta, register_runtime_event_envelope_meta,
    };
}
