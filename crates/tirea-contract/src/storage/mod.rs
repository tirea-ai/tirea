//! Persistent Thread storage contracts and abstractions.

pub mod traits;
pub mod types;

pub use traits::{ThreadReader, ThreadStore, ThreadSync, ThreadWriter};
pub use types::{
    paginate_in_memory, Committed, MessagePage, MessageQuery, MessageWithCursor, SortOrder,
    ThreadHead, ThreadListPage, ThreadListQuery, ThreadStoreError, VersionPrecondition,
};
