use carve_agent_contract::runtime::{Interaction, InteractionResponse};
use carve_agent_contract::state::ToolCall;
use carve_state_derive::State;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

pub const AGENT_STATE_PATH: &str = "agent";
pub const AGENT_RECOVERY_INTERACTION_ACTION: &str = "recover_agent_run";
pub const AGENT_RECOVERY_INTERACTION_PREFIX: &str = "agent_recovery_";

#[derive(Debug, Clone, Default, Serialize, Deserialize, State)]
pub struct AgentControlState {
    #[carve(default = "None")]
    pub pending_interaction: Option<Interaction>,
    #[carve(default = "Vec::new()")]
    pub replay_tool_calls: Vec<ToolCall>,
    #[carve(default = "Vec::new()")]
    pub interaction_resolutions: Vec<InteractionResponse>,
    #[carve(default = "HashMap::new()")]
    pub append_user_messages: HashMap<String, Vec<String>>,
}
