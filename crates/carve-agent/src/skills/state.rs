use carve_state_derive::State;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

pub const SKILLS_STATE_PATH: &str = "skills";

/// Persisted skill state (instructions + loaded materials).
///
/// This is designed for progressive disclosure:
/// - metadata is discovered via `SkillRegistry` (in-memory)
/// - instructions are stored when the skill is activated
/// - references/scripts are stored when explicitly loaded/executed
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
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LoadedReference {
    pub skill: String,
    pub path: String,
    pub sha256: String,
    pub truncated: bool,
    pub content: String,
    pub bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ScriptResult {
    pub skill: String,
    pub script: String,
    pub sha256: String,
    pub truncated_stdout: bool,
    pub truncated_stderr: bool,
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
}

pub fn material_key(skill_id: &str, relative_path: &str) -> String {
    format!("{skill_id}:{relative_path}")
}
