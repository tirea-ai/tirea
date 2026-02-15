use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkillMeta {
    pub id: String,
    pub name: String,
    pub description: String,
    pub allowed_tools: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkillRegistryWarning {
    pub path: PathBuf,
    pub reason: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SkillResourceKind {
    Reference,
    Asset,
}

impl SkillResourceKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Reference => "reference",
            Self::Asset => "asset",
        }
    }
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LoadedAsset {
    pub skill: String,
    pub path: String,
    pub sha256: String,
    pub truncated: bool,
    pub bytes: u64,
    pub media_type: Option<String>,
    pub encoding: String,
    pub content: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "resource", rename_all = "snake_case")]
pub enum SkillResource {
    Reference(LoadedReference),
    Asset(LoadedAsset),
}

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
pub enum SkillRegistryError {
    #[error("unknown skill: {0}")]
    UnknownSkill(String),

    #[error("invalid SKILL.md: {0}")]
    InvalidSkillMd(String),

    #[error("materialize error: {0}")]
    Materialize(#[from] SkillMaterializeError),

    #[error("io error: {0}")]
    Io(String),

    #[error("duplicate skill id: {0}")]
    DuplicateSkillId(String),

    #[error("unsupported operation: {0}")]
    Unsupported(String),
}

#[async_trait]
pub trait SkillRegistry: Send + Sync + std::fmt::Debug {
    fn list(&self) -> Vec<SkillMeta>;

    fn warnings(&self) -> Vec<SkillRegistryWarning> {
        Vec::new()
    }

    fn get(&self, skill_id: &str) -> Option<SkillMeta>;

    fn resolve(&self, key: &str) -> Option<SkillMeta> {
        self.get(key.trim())
    }

    async fn read_skill_md(&self, skill_id: &str) -> Result<String, SkillRegistryError>;

    async fn load_resource(
        &self,
        skill_id: &str,
        kind: SkillResourceKind,
        path: &str,
    ) -> Result<SkillResource, SkillRegistryError>;

    async fn run_script(
        &self,
        skill_id: &str,
        script: &str,
        args: &[String],
    ) -> Result<ScriptResult, SkillRegistryError>;
}

#[derive(Debug, thiserror::Error)]
pub enum CompositeSkillRegistryError {
    #[error("duplicate skill id: {0}")]
    DuplicateSkillId(String),
}

/// Combine multiple registries into one, failing fast on duplicate skill ids.
#[derive(Debug, Clone)]
pub struct CompositeSkillRegistry {
    sources: Vec<Arc<dyn SkillRegistry>>,
    metas: Vec<SkillMeta>,
    by_id: HashMap<String, (SkillMeta, usize)>,
    warnings: Vec<SkillRegistryWarning>,
}

impl CompositeSkillRegistry {
    pub fn new(sources: Vec<Arc<dyn SkillRegistry>>) -> Result<Self, CompositeSkillRegistryError> {
        let mut metas: Vec<SkillMeta> = Vec::new();
        let mut by_id: HashMap<String, (SkillMeta, usize)> = HashMap::new();
        let mut warnings: Vec<SkillRegistryWarning> = Vec::new();

        for (idx, reg) in sources.iter().enumerate() {
            warnings.extend(reg.warnings());
            for meta in reg.list() {
                if by_id.contains_key(&meta.id) {
                    return Err(CompositeSkillRegistryError::DuplicateSkillId(meta.id));
                }
                by_id.insert(meta.id.clone(), (meta.clone(), idx));
                metas.push(meta);
            }
        }

        metas.sort_by(|a, b| a.id.cmp(&b.id));

        Ok(Self {
            sources,
            metas,
            by_id,
            warnings,
        })
    }

    fn source_for(&self, skill_id: &str) -> Result<Arc<dyn SkillRegistry>, SkillRegistryError> {
        let (_, idx) = self
            .by_id
            .get(skill_id)
            .cloned()
            .ok_or_else(|| SkillRegistryError::UnknownSkill(skill_id.to_string()))?;
        Ok(self.sources[idx].clone())
    }
}

#[async_trait]
impl SkillRegistry for CompositeSkillRegistry {
    fn list(&self) -> Vec<SkillMeta> {
        self.metas.clone()
    }

    fn warnings(&self) -> Vec<SkillRegistryWarning> {
        self.warnings.clone()
    }

    fn get(&self, skill_id: &str) -> Option<SkillMeta> {
        self.by_id.get(skill_id).map(|(m, _)| m.clone())
    }

    async fn read_skill_md(&self, skill_id: &str) -> Result<String, SkillRegistryError> {
        self.source_for(skill_id)?.read_skill_md(skill_id).await
    }

    async fn load_resource(
        &self,
        skill_id: &str,
        kind: SkillResourceKind,
        path: &str,
    ) -> Result<SkillResource, SkillRegistryError> {
        self.source_for(skill_id)?
            .load_resource(skill_id, kind, path)
            .await
    }

    async fn run_script(
        &self,
        skill_id: &str,
        script: &str,
        args: &[String],
    ) -> Result<ScriptResult, SkillRegistryError> {
        self.source_for(skill_id)?
            .run_script(skill_id, script, args)
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug)]
    struct MockRegistry {
        metas: Vec<SkillMeta>,
        docs: HashMap<String, String>,
    }

    impl MockRegistry {
        fn new(metas: Vec<SkillMeta>, docs: HashMap<String, String>) -> Self {
            Self { metas, docs }
        }
    }

    #[async_trait]
    impl SkillRegistry for MockRegistry {
        fn list(&self) -> Vec<SkillMeta> {
            self.metas.clone()
        }

        fn get(&self, skill_id: &str) -> Option<SkillMeta> {
            self.metas.iter().find(|m| m.id == skill_id).cloned()
        }

        async fn read_skill_md(&self, skill_id: &str) -> Result<String, SkillRegistryError> {
            self.docs
                .get(skill_id)
                .cloned()
                .ok_or_else(|| SkillRegistryError::UnknownSkill(skill_id.to_string()))
        }

        async fn load_resource(
            &self,
            _skill_id: &str,
            _kind: SkillResourceKind,
            _path: &str,
        ) -> Result<SkillResource, SkillRegistryError> {
            Err(SkillRegistryError::Unsupported(
                "resource loading not mocked".to_string(),
            ))
        }

        async fn run_script(
            &self,
            _skill_id: &str,
            _script: &str,
            _args: &[String],
        ) -> Result<ScriptResult, SkillRegistryError> {
            Err(SkillRegistryError::Unsupported(
                "script execution not mocked".to_string(),
            ))
        }
    }

    fn meta(id: &str) -> SkillMeta {
        SkillMeta {
            id: id.to_string(),
            name: id.to_string(),
            description: format!("{id} skill"),
            allowed_tools: Vec::new(),
        }
    }

    #[tokio::test]
    async fn composite_registry_delegates_by_skill_id_and_sorts_list() {
        let reg_a = Arc::new(MockRegistry::new(
            vec![meta("alpha")],
            HashMap::from([(String::from("alpha"), String::from("doc-a"))]),
        ));
        let reg_b = Arc::new(MockRegistry::new(
            vec![meta("beta")],
            HashMap::from([(String::from("beta"), String::from("doc-b"))]),
        ));
        let composite = CompositeSkillRegistry::new(vec![reg_b, reg_a]).expect("compose");

        let ids: Vec<String> = composite.list().into_iter().map(|m| m.id).collect();
        assert_eq!(ids, vec!["alpha".to_string(), "beta".to_string()]);
        assert_eq!(
            composite.read_skill_md("alpha").await.expect("alpha doc"),
            "doc-a"
        );
        assert_eq!(
            composite.read_skill_md("beta").await.expect("beta doc"),
            "doc-b"
        );
    }

    #[test]
    fn composite_registry_rejects_duplicate_skill_ids() {
        let reg1 = Arc::new(MockRegistry::new(vec![meta("dup")], HashMap::new()));
        let reg2 = Arc::new(MockRegistry::new(vec![meta("dup")], HashMap::new()));
        let err = CompositeSkillRegistry::new(vec![reg1, reg2]).expect_err("must fail");
        assert!(matches!(
            err,
            CompositeSkillRegistryError::DuplicateSkillId(ref id) if id == "dup"
        ));
    }

    #[test]
    fn skill_registry_error_preserves_materialize_variant() {
        let err: SkillRegistryError = SkillMaterializeError::PathEscapesRoot.into();
        assert!(matches!(
            err,
            SkillRegistryError::Materialize(SkillMaterializeError::PathEscapesRoot)
        ));
    }
}
