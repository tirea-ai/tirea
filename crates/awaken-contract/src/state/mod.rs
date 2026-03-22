mod slot;
mod snapshot;

pub use slot::{KeyScope, MergeStrategy, StateKey, StateKeyOptions, StateMap};
pub use snapshot::{PersistedState, Snapshot};
