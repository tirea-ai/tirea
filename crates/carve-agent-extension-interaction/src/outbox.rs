//! Interaction outbox — persisted queue for replay calls and resolution events.
//!
//! Stored at `state["interaction_outbox"]`. Written by interaction plugins,
//! drained by the agent loop.

use carve_agent_contract::runtime::interaction::InteractionResponse;
use carve_agent_contract::state::ToolCall;
use carve_state::State;
use serde::{Deserialize, Serialize};

/// JSON path under `AgentState.state` where the interaction outbox is stored.
pub const INTERACTION_OUTBOX_PATH: &str = "interaction_outbox";

/// Persisted outbox for interaction-driven tool replay and resolution events.
#[derive(Debug, Clone, Default, Serialize, Deserialize, State)]
pub struct InteractionOutbox {
    /// Tool calls to replay at session start after an interaction is approved.
    #[carve(default = "Vec::new()")]
    pub replay_tool_calls: Vec<ToolCall>,
    /// Interaction responses to emit as runtime events.
    #[carve(default = "Vec::new()")]
    pub interaction_resolutions: Vec<InteractionResponse>,
}
