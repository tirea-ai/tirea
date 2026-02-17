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
pub mod events {
    pub use crate::{AgentEvent, RunRequest, StopReason, StreamResult, TerminationReason};
}
pub mod phase {
    pub use crate::extension::phase::*;
}
pub mod agent_plugin {
    pub use crate::extension::agent_plugin::*;
}
pub mod state_types {
    pub use crate::extension::state_types::*;
}
pub mod traits {
    pub use crate::extension::traits::{provider, reminder, tool};
}
pub mod tools {
    pub use crate::extension::traits::tool::{
        Tool, ToolDescriptor, ToolError, ToolResult, ToolStatus,
    };
}
pub mod plugin {
    pub use crate::agent_plugin::AgentPlugin;
    pub use crate::phase::{Phase, StepContext, StepOutcome, ToolContext};
    pub use crate::traits::provider::{ContextCategory, ContextProvider};
    pub use crate::traits::reminder::SystemReminder;
}
pub mod state {
    pub use crate::state_types::{
        AGENT_RECOVERY_INTERACTION_ACTION, AGENT_RECOVERY_INTERACTION_PREFIX, AGENT_STATE_PATH,
        AgentRunState, AgentRunStatus, AgentState, Interaction, InteractionResponse,
        ToolPermissionBehavior,
    };
}

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
