//! Persistent AgentState storage contracts and abstractions.

pub mod traits;
pub mod types;

pub use traits::{ThreadReader, ThreadStore, ThreadSync, ThreadWriter};
pub use types::{
    AgentChangeSet, AgentStateHead, CheckpointReason, Committed, MessagePage, MessageQuery,
    MessageWithCursor, SortOrder, ThreadListPage,
    ThreadListQuery, ThreadStoreError, Version, paginate_in_memory,
};
