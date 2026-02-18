//! Shared agent contracts for conversation state, runtime protocol, extension SPI, and storage.
#![allow(missing_docs)]

pub mod plugin;
pub mod runtime;
pub mod state;
pub mod storage;
pub mod tool;
pub mod tool_registry;

pub use plugin::AgentPlugin;
pub use runtime::{
    AgentEvent, AgentRunsState, InferenceError, Interaction, InteractionResponse, LlmExecutor,
    RunRequest, RunState, RunStatus, RuntimeControlExt, RuntimeControlState, StopConditionSpec,
    StopPolicy, StopPolicyInput, StopPolicyStats, StopReason, StreamResult, TerminationReason,
    ToolExecution, ToolExecutionRequest, ToolExecutionResult, ToolExecutor, ToolExecutorError,
    AGENT_RECOVERY_INTERACTION_ACTION, AGENT_RECOVERY_INTERACTION_PREFIX, AGENT_RUNS_STATE_PATH,
    RUNTIME_CONTROL_STATE_PATH,
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
