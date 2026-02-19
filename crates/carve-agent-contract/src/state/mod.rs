//! Agent state model: persistent data, changesets, and transient execution context.

pub mod changeset;
pub mod message;
pub mod model;
pub mod transient;

pub use changeset::{ThreadChangeSet, CheckpointReason, Version};
pub use message::{gen_message_id, Message, MessageMetadata, Role, ToolCall, Visibility};
pub use model::{PendingDelta, Thread, ThreadMetadata};
pub use crate::context::ActivityContext;
pub use transient::ActivityManager;
