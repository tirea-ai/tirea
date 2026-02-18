//! Runtime control-state — re-exported from [`carve_agent_contract::runtime::control`].

pub use crate::contracts::runtime::control::{
    InferenceError, RuntimeControlExt, RuntimeControlState, AGENT_RECOVERY_INTERACTION_ACTION,
    AGENT_RECOVERY_INTERACTION_PREFIX, RUNTIME_CONTROL_STATE_PATH,
};
