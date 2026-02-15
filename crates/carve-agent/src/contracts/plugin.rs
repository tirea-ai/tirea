//! Plugin contracts.

pub use super::agent_plugin::AgentPlugin;
pub use super::phase::{Phase, StepContext, StepOutcome, ToolContext};
pub use super::traits::provider::{ContextCategory, ContextProvider};
pub use super::traits::reminder::SystemReminder;
