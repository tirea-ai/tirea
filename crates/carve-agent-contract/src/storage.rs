//! Persistent AgentState storage contracts and abstractions.

pub mod traits;
pub mod types;

pub use traits::{AgentStateReader, AgentStateStore, AgentStateSync, AgentStateWriter};
pub use types::{
    AgentChangeSet, AgentStateHead, CheckpointReason, Committed, MessagePage, MessageQuery,
    MessageWithCursor, SortOrder, AgentStateListPage,
    AgentStateListQuery, AgentStateStoreError, Version, paginate_in_memory,
};
