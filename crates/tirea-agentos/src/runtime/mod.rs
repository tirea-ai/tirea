pub(crate) mod agent_tools;
mod behavior;
mod bundle_merge;
mod errors;
pub(crate) mod plugin;
mod policy;
mod prepare;
pub(crate) mod resolve;
mod run;
pub(crate) mod thread_run;
mod types;

#[cfg(test)]
mod tests;

pub use behavior::compose_behaviors;
pub use errors::{AgentOsResolveError, AgentOsRunError};
pub use plugin::context_window::{ContextWindowPlugin, CONTEXT_WINDOW_PLUGIN_ID};
pub use plugin::stop_policy::{
    ConsecutiveErrors, ContentMatch, LoopDetection, MaxRounds, StopPolicy, StopPolicyInput,
    StopPolicyPlugin, StopPolicyStats, StopOnTool, Timeout, TokenBudget,
};
pub use thread_run::ForwardedDecision;
pub use types::{AgentOs, PreparedRun, RunStream};

pub(crate) use types::RuntimeServices;

pub use crate::loop_runtime::loop_runner::ResolvedRun;

#[cfg(test)]
pub(crate) use crate::composition::AgentDefinition;
