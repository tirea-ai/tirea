//! Persistent Thread storage contracts and abstractions.

pub mod traits;
pub mod types;

pub use traits::{AgentStateReader, AgentStateStore, AgentStateSync, AgentStateWriter};
pub use types::{
    paginate_in_memory, AgentStateHead, AgentStateListPage, AgentStateListQuery,
    AgentStateStoreError, Committed, MessagePage, MessageQuery, MessageWithCursor, SortOrder,
    VersionPrecondition,
};
