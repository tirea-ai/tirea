//! AgentState store contract and shared persistence/query types.

use async_trait::async_trait;
use carve_state::TrackedPatch;
pub use carve_agent_contract::{AgentState, Message, MessageMetadata, Role, Visibility};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Arc;
use thiserror::Error;

mod traits;
mod types;

pub use traits::{ThreadReader, ThreadStore, ThreadSync, ThreadWriter};
pub use types::{
    paginate_in_memory, CheckpointReason, Committed, MessagePage, MessageQuery, MessageWithCursor,
    SortOrder, AgentChangeSet, AgentStateHead, ThreadListPage, ThreadListQuery,
    ThreadStoreError, Version,
};
