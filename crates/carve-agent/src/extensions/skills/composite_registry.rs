use crate::extensions::skills::registry::{
    SkillRegistry, SkillRegistryError, SkillRegistryWarning, SkillResource, SkillResourceKind,
};
use crate::extensions::skills::state::ScriptResult;
use crate::extensions::skills::SkillMeta;
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Arc;

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
    use crate::extensions::skills::in_memory_registry::InMemorySkillRegistry;
    use crate::extensions::skills::SkillMeta;
    use genai::Client;

    #[tokio::test]
    async fn composite_delegates_to_correct_source() {
        let mut a = InMemorySkillRegistry::new();
        a.insert_skill(
            SkillMeta {
                id: "a".to_string(),
                name: "a".to_string(),
                description: "a".to_string(),
                allowed_tools: vec![],
            },
            "---\nname: a\ndescription: a\n---\nBody A\n",
        );

        let mut b = InMemorySkillRegistry::new();
        b.insert_skill(
            SkillMeta {
                id: "b".to_string(),
                name: "b".to_string(),
                description: "b".to_string(),
                allowed_tools: vec![],
            },
            "---\nname: b\ndescription: b\n---\nBody B\n",
        );

        let reg = CompositeSkillRegistry::new(vec![Arc::new(a), Arc::new(b)]).unwrap();
        assert_eq!(reg.list().len(), 2);
        assert!(reg.get("a").is_some());
        assert!(reg.get("b").is_some());

        let md = reg.read_skill_md("b").await.unwrap();
        assert!(md.contains("Body B"));

        // Ensure composite can be used behind dyn SkillRegistry.
        let _as_dyn: Arc<dyn SkillRegistry> = Arc::new(reg);
        let _client = Client::default();
    }

    #[test]
    fn composite_rejects_duplicate_ids() {
        let mut a = InMemorySkillRegistry::new();
        a.insert_skill(
            SkillMeta {
                id: "x".to_string(),
                name: "x".to_string(),
                description: "x".to_string(),
                allowed_tools: vec![],
            },
            "---\nname: x\ndescription: x\n---\nBody\n",
        );
        let b = a.clone();

        let err = CompositeSkillRegistry::new(vec![Arc::new(a), Arc::new(b)])
            .err()
            .unwrap();
        assert!(matches!(err, CompositeSkillRegistryError::DuplicateSkillId(ref id) if id == "x"));
    }
}
