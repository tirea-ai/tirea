//! Shared runtime contract types for agent execution and protocol adapters.

mod event;
mod interaction;
mod run;
mod stop;
mod tool;

pub use event::AgentEvent;
pub use interaction::{Interaction, InteractionResponse};
pub use run::RunRequest;
pub use stop::StopReason;
pub use tool::{ToolResult, ToolStatus};
