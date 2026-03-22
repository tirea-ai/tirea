use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum SkillMaterializeError {
    #[error("invalid relative path: {0}")]
    InvalidPath(String),

    #[error("path is outside skill root")]
    PathEscapesRoot,

    #[error("unsupported path (expected under {0})")]
    UnsupportedPath(String),

    #[error("io error: {0}")]
    Io(String),

    #[error("script runtime not supported for: {0}")]
    UnsupportedRuntime(String),

    #[error("script timed out after {0}s")]
    Timeout(u64),

    #[error("invalid script arguments: {0}")]
    InvalidScriptArgs(String),
}

#[derive(Debug, thiserror::Error)]
pub enum SkillError {
    #[error("unknown skill: {0}")]
    UnknownSkill(String),

    #[error("invalid SKILL.md: {0}")]
    InvalidSkillMd(String),

    #[error("invalid skill arguments: {0}")]
    InvalidArguments(String),

    #[error("materialize error: {0}")]
    Materialize(#[from] SkillMaterializeError),

    #[error("io error: {0}")]
    Io(String),

    #[error("duplicate skill id: {0}")]
    DuplicateSkillId(String),

    #[error("unsupported operation: {0}")]
    Unsupported(String),
}

#[derive(Debug, thiserror::Error)]
pub enum SkillRegistryError {
    #[error("skill id must be non-empty")]
    EmptySkillId,

    #[error("duplicate skill id: {0}")]
    DuplicateSkillId(String),
}

#[derive(Debug, thiserror::Error)]
pub enum SkillRegistryManagerError {
    #[error(transparent)]
    Skill(#[from] SkillError),

    #[error(transparent)]
    Registry(#[from] SkillRegistryError),

    #[error("skill roots list must be non-empty")]
    EmptyRoots,

    #[error("periodic refresh interval must be > 0")]
    InvalidRefreshInterval,

    #[error("periodic refresh loop is already running")]
    PeriodicRefreshAlreadyRunning,

    #[error("periodic refresh join failed: {0}")]
    Join(String),
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SkillWarning {
    pub path: PathBuf,
    pub reason: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skill_error_preserves_materialize_variant() {
        let err: SkillError = SkillMaterializeError::PathEscapesRoot.into();
        assert!(matches!(
            err,
            SkillError::Materialize(SkillMaterializeError::PathEscapesRoot)
        ));
    }
}
