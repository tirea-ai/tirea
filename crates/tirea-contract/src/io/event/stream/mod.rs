mod definition;
mod envelope_meta;
#[cfg(test)]
mod tests;
mod wire;

pub use definition::AgentEvent;

#[doc(hidden)]
pub mod internal {
    pub use super::envelope_meta::{
        clear_runtime_event_envelope_meta, register_runtime_event_envelope_meta,
    };
}
