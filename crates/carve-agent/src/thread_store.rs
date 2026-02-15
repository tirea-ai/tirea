//! Storage backend traits for thread persistence.

use crate::thread::Thread;
use crate::types::{Message, Visibility};
use async_trait::async_trait;
use carve_state::TrackedPatch;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Arc;
use thiserror::Error;

mod memory_store;
mod traits;
mod types;

pub use memory_store::MemoryStore;
pub use traits::{ThreadReader, ThreadStore, ThreadSync, ThreadWriter};
pub use types::{
    paginate_in_memory, CheckpointReason, Committed, MessagePage, MessageQuery, MessageWithCursor,
    SortOrder, ThreadDelta, ThreadHead, ThreadListPage, ThreadListQuery, ThreadStoreError, Version,
};

#[cfg(test)]
mod tests;
