//! Shared agent contracts for conversation state, runtime protocol, extension SPI, and storage.

pub mod agent;
pub mod change;
pub mod composition;
pub mod conversation;
pub mod extension;
pub mod runtime;
pub mod skills;
pub mod stop_conditions;
pub mod storage;

pub use agent::{
    AgentConfig, AgentDefinition, LlmRetryPolicy, RunCancellationToken, RunContext,
    StateCommitError, StateCommitter, TOOL_SCOPE_CALLER_AGENT_ID_KEY,
    TOOL_SCOPE_CALLER_MESSAGES_KEY, TOOL_SCOPE_CALLER_STATE_KEY, TOOL_SCOPE_CALLER_THREAD_ID_KEY,
};
pub use change::{AgentChangeSet, CheckpointChangeSet, CheckpointReason, Version};
pub use conversation::{
    gen_message_id, AgentState, AgentStateMetadata, Message, MessageMetadata, PendingDelta, Role,
    ToolCall, Visibility,
};
pub use extension::persisted_state::{
    AgentInferenceError, AgentRunState, AgentRunStatus, PersistedAgentState,
    ToolPermissionBehavior, AGENT_RECOVERY_INTERACTION_ACTION, AGENT_RECOVERY_INTERACTION_PREFIX,
    AGENT_STATE_PATH,
};
pub use extension::plugin::AgentPlugin;
pub use extension::traits::provider::{ContextCategory, ContextProvider};
pub use extension::traits::reminder::SystemReminder;
pub use extension::traits::tool::{Tool, ToolDescriptor, ToolError};
pub use runtime::{
    ActivityContext, ActivityManager, AgentEvent, Interaction, InteractionResponse, Phase,
    RunRequest, StepContext, StepOutcome, StopReason, StreamResult, TerminationReason, ToolContext,
    ToolResult, ToolStatus,
};
pub use storage::*;
