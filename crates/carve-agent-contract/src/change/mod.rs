//! Change-set contracts for AgentState persistence and checkpointing.

mod checkpoint;
mod delta;

pub use checkpoint::CheckpointChangeSet;
pub use delta::{AgentChangeSet, CheckpointReason, Version};
