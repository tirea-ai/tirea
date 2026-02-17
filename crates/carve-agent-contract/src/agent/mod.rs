//! Agent configuration and runtime lifecycle contracts.

mod definition;
pub mod stop;

pub use definition::{AgentConfig, AgentDefinition, LlmRetryPolicy};
pub use stop::{
    ConsecutiveErrors, ContentMatch, LoopDetection, MaxRounds, StopCheckContext, StopCondition,
    StopConditionSpec, StopOnTool, Timeout, TokenBudget,
};
