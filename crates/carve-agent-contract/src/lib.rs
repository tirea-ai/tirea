//! Shared agent contracts for conversation state, runtime protocol, extension SPI, and storage.

pub mod plugin;
pub mod runtime;
pub mod state;
pub mod storage;
pub mod tool;
pub mod tool_registry;

pub use plugin::AgentPlugin;
pub use runtime::{
    AgentEvent, Interaction, InteractionResponse, LlmExecutor, RunRequest, StopConditionSpec,
    StopPolicy, StopPolicyInput, StopPolicyStats, StopReason, StreamResult, TerminationReason,
    ToolExecution, ToolExecutionRequest, ToolExecutionResult, ToolExecutor, ToolExecutorError,
};
pub use state::{
    gen_message_id, AgentChangeSet, AgentState, AgentStateMetadata, CheckpointReason, Message,
    MessageMetadata, PendingDelta, Role, ToolCall, Version, Visibility,
};
pub use storage::{
    paginate_in_memory, AgentStateHead, AgentStateListPage, AgentStateListQuery, AgentStateReader,
    AgentStateStore, AgentStateStoreError, AgentStateSync, AgentStateWriter, Committed,
    MessagePage, MessageQuery, MessageWithCursor, SortOrder, VersionPrecondition,
};
pub use tool::{Tool, ToolDescriptor, ToolError, ToolResult, ToolStatus};
pub use tool_registry::{ToolRegistry, ToolRegistryError};
