use carve_agent_contract::skills::{LoadedAsset, LoadedReference, ScriptResult};
use carve_state_derive::State;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

pub const SKILLS_STATE_PATH: &str = "skills";

/// Persisted skill state (instructions + loaded materials).
#[derive(Debug, Clone, Default, Serialize, Deserialize, State)]
pub struct SkillState {
    /// Activated skill IDs (stable identifiers from the registry).
    #[serde(default)]
    pub active: Vec<String>,
    /// Activated skill instructions (SKILL.md body), keyed by skill ID.
    #[serde(default)]
    pub instructions: HashMap<String, String>,
    /// Loaded references, keyed by `<skill_id>:<relative_path>`.
    #[serde(default)]
    pub references: HashMap<String, LoadedReference>,
    /// Script results, keyed by `<skill_id>:<relative_path>`.
    #[serde(default)]
    pub scripts: HashMap<String, ScriptResult>,
    /// Loaded assets, keyed by `<skill_id>:<relative_path>`.
    #[serde(default)]
    pub assets: HashMap<String, LoadedAsset>,
}

pub fn material_key(skill_id: &str, relative_path: &str) -> String {
    format!("{skill_id}:{relative_path}")
}
