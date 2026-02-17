//! Shared agent contracts for runtime events, extension SPI, and composition wiring.

pub mod context;
mod event;
mod interaction;
pub mod conversation;
mod run;
pub mod change;
mod stop;
mod stream;
mod tool;
pub mod storage;

pub mod agent;
pub mod composition;
pub mod extension;
pub mod skills;
pub mod stop_conditions;

pub use change::CheckpointReason;
pub use conversation::{
    AgentState, AgentStateMetadata, PendingDelta, ToolCall, Visibility, gen_message_id, Message,
    MessageMetadata, Role,
};
pub use context::{ActivityContext, ActivityManager, AgentChangeSet};
pub use event::AgentEvent;
pub use interaction::{Interaction, InteractionResponse};
pub use run::RunRequest;
pub use stop::{StopReason, TerminationReason};
pub use stream::StreamResult;
pub use tool::{ToolResult, ToolStatus};
pub use storage::*;
