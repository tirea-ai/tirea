//! Shared agent contracts for conversation state, runtime protocol, extension SPI, and storage.

pub mod agent;
pub mod composition;
pub mod extension;
pub mod runtime;
pub mod state;
pub mod storage;

pub use agent::{
    check_stop_conditions, AgentConfig, AgentDefinition, ConsecutiveErrors, ContentMatch,
    LlmRetryPolicy, LoopDetection, MaxRounds, RunCancellationToken, RunContext, StateCommitError,
    StateCommitter, StopCheckContext, StopCondition, StopConditionSpec, StopOnTool, Timeout,
    TokenBudget, TOOL_SCOPE_CALLER_AGENT_ID_KEY, TOOL_SCOPE_CALLER_MESSAGES_KEY,
    TOOL_SCOPE_CALLER_STATE_KEY, TOOL_SCOPE_CALLER_THREAD_ID_KEY,
};
pub use extension::permission::{PermissionState, ToolPermissionBehavior, PERMISSION_STATE_PATH};
pub use extension::plugin::AgentPlugin;
pub use extension::skills::{
    material_key, CompositeSkillRegistry, CompositeSkillRegistryError, LoadedAsset,
    LoadedReference, ScriptResult, SkillMaterializeError, SkillMeta, SkillRegistry,
    SkillRegistryError, SkillRegistryWarning, SkillResource, SkillResourceKind, SkillState,
    SKILLS_STATE_PATH,
};
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
