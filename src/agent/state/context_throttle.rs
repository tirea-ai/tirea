use crate::state::StateKey;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Per-key throttle entry: tracks when a context message was last injected.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThrottleEntry {
    /// Step number when this key was last injected.
    pub last_step: usize,
    /// Hash of the content at last injection (re-inject if content changes).
    pub content_hash: u64,
}

/// Throttle state for context message injection.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContextThrottleMap {
    pub entries: HashMap<String, ThrottleEntry>,
}

/// Update for the context throttle state.
pub enum ContextThrottleUpdate {
    /// Record that a key was injected at a given step with a content hash.
    Injected {
        key: String,
        step: usize,
        content_hash: u64,
    },
}

/// State key for context message throttle tracking.
///
/// Tracks per-key injection history so the loop runner can enforce cooldown rules.
pub struct ContextThrottleState;

impl StateKey for ContextThrottleState {
    const KEY: &'static str = "__runtime.context_throttle";

    type Value = ContextThrottleMap;
    type Update = ContextThrottleUpdate;

    fn apply(value: &mut Self::Value, update: Self::Update) {
        match update {
            ContextThrottleUpdate::Injected {
                key,
                step,
                content_hash,
            } => {
                value.entries.insert(
                    key,
                    ThrottleEntry {
                        last_step: step,
                        content_hash,
                    },
                );
            }
        }
    }
}
