//! Storage backend traits for thread persistence.

use crate::thread::Thread;
use crate::types::{Message, Visibility};
use async_trait::async_trait;
use carve_state::TrackedPatch;
use serde::{Deserialize, Serialize};
use serde_json::Value;
#[cfg(feature = "postgres")]
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;
use thiserror::Error;
use tokio::io::AsyncWriteExt;

mod file_store;
mod memory_store;
#[cfg(feature = "postgres")]
mod postgres_store;
mod traits;
mod types;

pub use file_store::FileStore;
pub use memory_store::MemoryStore;
#[cfg(feature = "postgres")]
pub use postgres_store::PostgresStore;
pub use traits::{ThreadReader, ThreadStore, ThreadSync, ThreadWriter};
pub use types::{
    paginate_in_memory, CheckpointReason, Committed, MessagePage, MessageQuery, MessageWithCursor,
    SortOrder, ThreadDelta, ThreadHead, ThreadListPage, ThreadListQuery, ThreadStoreError, Version,
};

#[cfg(test)]
mod tests;
