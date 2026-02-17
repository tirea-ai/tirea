//! Agent configuration and runtime lifecycle contracts.

mod definition;
mod run_context;

pub use definition::{AgentConfig, AgentDefinition, LlmRetryPolicy};
pub use run_context::{
    RunCancellationToken, RunContext, StateCommitError, StateCommitter,
    TOOL_SCOPE_CALLER_AGENT_ID_KEY, TOOL_SCOPE_CALLER_MESSAGES_KEY, TOOL_SCOPE_CALLER_STATE_KEY,
    TOOL_SCOPE_CALLER_THREAD_ID_KEY,
};
