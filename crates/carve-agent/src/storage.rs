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

mod file;
mod memory;
#[cfg(feature = "postgres")]
mod postgres;
mod traits;
mod types;

pub use file::FileStorage;
pub use memory::MemoryStorage;
#[cfg(feature = "postgres")]
pub use postgres::PostgresStorage;
pub use traits::{ThreadQuery, ThreadStore, ThreadSync};
pub use types::{
    paginate_in_memory, CheckpointReason, Committed, MessagePage, MessageQuery, MessageWithCursor,
    SortOrder, StorageError, ThreadDelta, ThreadHead, ThreadListPage, ThreadListQuery, Version,
};

#[cfg(test)]
mod tests;
