//! Thread conversation domain model.

pub mod thread;
pub mod types;

pub use thread::{PendingDelta, Thread, ThreadMetadata};
pub use types::{gen_message_id, Message, MessageMetadata, Role, ToolCall, Visibility};
