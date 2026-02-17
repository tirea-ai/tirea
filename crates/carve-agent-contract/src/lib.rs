//! Shared agent contracts for conversation state, runtime protocol, extension SPI, and storage.

pub mod agent;
pub mod composition;
pub mod extension;
pub mod runtime;
pub mod state;
pub mod storage;

pub use agent::{
    AgentConfig, AgentDefinition, ConsecutiveErrors, ContentMatch, LlmRetryPolicy, LoopDetection,
    MaxRounds, StopCheckContext, StopCondition, StopConditionSpec, StopOnTool, Timeout,
    TokenBudget,
};
pub use extension::plugin::AgentPlugin;
pub use extension::traits::provider::{ContextCategory, ContextProvider};
pub use extension::traits::reminder::SystemReminder;
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
