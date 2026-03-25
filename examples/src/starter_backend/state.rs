use awaken_contract::state::{KeyScope, MergeStrategy, StateKey};
use serde::{Deserialize, Serialize};

/// Root state for the starter backend demo.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct StarterStateValue {
    #[serde(default)]
    pub notes: Vec<String>,
    #[serde(default)]
    pub theme_color: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum StarterAction {
    SetNotes(Vec<String>),
}

/// State key binding for the starter backend.
pub struct StarterState;

impl StateKey for StarterState {
    const KEY: &'static str = "starter";
    const MERGE: MergeStrategy = MergeStrategy::Exclusive;
    const SCOPE: KeyScope = KeyScope::Run;

    type Value = StarterStateValue;
    type Update = StarterAction;

    fn apply(value: &mut Self::Value, update: Self::Update) {
        match update {
            StarterAction::SetNotes(notes) => value.notes = notes,
        }
    }
}
