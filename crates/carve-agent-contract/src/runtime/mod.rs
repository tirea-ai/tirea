//! Runtime protocol contracts: request, events, phase control, and outcomes.

pub mod control;
pub mod event;
pub mod executor;
pub mod interaction;
pub mod phase;
pub mod policy;
pub mod policy_scope;
pub mod request;
pub mod result;
pub mod termination;

pub use crate::state::{ActivityContext, ActivityManager};
pub use control::{
    AgentRunsState, InferenceError, RunState, RunStatus, RuntimeControlExt, RuntimeControlState,
    AGENT_RECOVERY_INTERACTION_ACTION, AGENT_RECOVERY_INTERACTION_PREFIX, AGENT_RUNS_STATE_PATH,
    RUNTIME_CONTROL_STATE_PATH,
};
pub use event::AgentEvent;
pub use executor::{
    LlmEventStream, LlmExecutor, ToolExecution, ToolExecutionRequest, ToolExecutionResult,
    ToolExecutor, ToolExecutorError,
};
pub use interaction::{Interaction, InteractionResponse};
pub use phase::{Phase, StepContext, StepOutcome, ToolContext};
pub use policy::{StopPolicy, StopPolicyInput, StopPolicyStats};
pub use policy_scope::{
    is_id_allowed, is_scope_allowed, parse_scope_filter, SCOPE_ALLOWED_AGENTS_KEY,
    SCOPE_ALLOWED_SKILLS_KEY, SCOPE_ALLOWED_TOOLS_KEY, SCOPE_EXCLUDED_AGENTS_KEY,
    SCOPE_EXCLUDED_SKILLS_KEY, SCOPE_EXCLUDED_TOOLS_KEY,
};
pub use request::RunRequest;
pub use result::{StreamResult, ToolResult, ToolStatus};
pub use termination::{StopConditionSpec, StopReason, TerminationReason};
