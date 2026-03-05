use serde::{Deserialize, Serialize};
use tirea_state_derive::State;

#[derive(Debug, Clone, Default, Serialize, Deserialize, State)]
pub struct StarterState {
    pub todos: Vec<String>,
    pub notes: Vec<String>,
    pub theme_color: String,
}
