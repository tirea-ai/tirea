//! Shared agent contracts for runtime events, extension SPI, and composition wiring.

mod context;
mod event;
mod interaction;
mod run;
mod stop;
mod stream;
mod tool;

pub mod agent;
pub mod composition;
pub mod extension;
pub mod skills;
pub mod stop_conditions;

pub use context::{ActivityContext, ActivityManager, AgentChangeSet, AgentState};
pub use event::AgentEvent;
pub use interaction::{Interaction, InteractionResponse};
pub use run::RunRequest;
pub use stop::{StopReason, TerminationReason};
pub use stream::StreamResult;
pub use tool::{ToolResult, ToolStatus};
