mod command;
mod mutation;
mod persistence;
mod store;

pub use command::StateCommand;
pub use mutation::MutationBatch;
pub use store::{CommitEvent, CommitHook, StateStore};

// Re-export contract state types for convenience
pub use awaken_contract::state::{
    KeyScope, MergeStrategy, PersistedState, Snapshot, StateKey, StateKeyOptions, StateMap,
};
