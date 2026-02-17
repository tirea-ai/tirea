//! Persistent AgentState storage contracts and abstractions.

pub mod traits;
pub mod types;

pub use traits::{AgentStateReader, AgentStateStore, AgentStateSync, AgentStateWriter};
pub use types::{
    paginate_in_memory, AgentChangeSet, AgentStateHead, AgentStateListPage, AgentStateListQuery,
    AgentStateStoreError, CheckpointReason, Committed, MessagePage, MessageQuery,
    MessageWithCursor, SortOrder, Version,
};
