//! Shared agent contracts for conversation state, runtime protocol, extension SPI, and storage.

pub mod composition;
pub mod extension;
pub mod runtime;
pub mod state;
pub mod storage;

pub use extension::plugin::AgentPlugin;
pub use extension::traits::tool::{Tool, ToolDescriptor, ToolError, ToolResult, ToolStatus};
pub use runtime::{
    AgentEvent, Interaction, InteractionResponse, RunRequest, StopReason, StreamResult,
    TerminationReason,
};
pub use state::{
    gen_message_id, AgentState, AgentStateMetadata, Message, MessageMetadata, PendingDelta, Role,
    ToolCall, Visibility,
};
pub use state::{AgentChangeSet, CheckpointReason, Version};
pub use storage::{
    paginate_in_memory, AgentStateHead, AgentStateListPage, AgentStateListQuery, AgentStateReader,
    AgentStateStore, AgentStateStoreError, AgentStateSync, AgentStateWriter, Committed,
    MessagePage, MessageQuery, MessageWithCursor, SortOrder, VersionPrecondition,
};
