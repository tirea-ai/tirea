//! Interaction outbox — persisted queue for replay calls and resolution events.
//!
//! Stored at `state["interaction_outbox"]`. Written by interaction plugins,
//! drained by the agent loop.

use tirea_contract::event::interaction::InteractionResponse;
use tirea_contract::thread::ToolCall;
use tirea_state::State;
use serde::{Deserialize, Serialize};

/// Persisted outbox for interaction-driven tool replay and resolution events.
#[derive(Debug, Clone, Default, Serialize, Deserialize, State)]
#[tirea(path = "interaction_outbox")]
pub struct InteractionOutbox {
    /// Tool calls to replay at session start after an interaction is approved.
    #[tirea(default = "Vec::new()")]
    pub replay_tool_calls: Vec<ToolCall>,
    /// Interaction responses to emit as runtime events.
    #[tirea(default = "Vec::new()")]
    pub interaction_resolutions: Vec<InteractionResponse>,
}
