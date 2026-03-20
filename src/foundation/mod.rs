mod error;
mod json;
mod runtime_types;
mod slots;
mod snapshot;

pub use error::{StateError, UnknownSlotPolicy};
pub use json::{JsonValue, decode_json, encode_json};
pub use runtime_types::*;
pub use slots::{SlotMap, SlotOptions, StateSlot};
pub use snapshot::{PersistedState, PluginMeta, Snapshot};
