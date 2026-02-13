use crate::skills::registry::{
    SkillRegistry, SkillRegistryError, SkillRegistryWarning, SkillResource, SkillResourceKind,
};
use crate::skills::state::{LoadedAsset, LoadedReference, ScriptResult};
use crate::skills::SkillMeta;
use async_trait::async_trait;
use std::collections::HashMap;

#[derive(Debug, Clone, Default)]
pub struct InMemorySkillRegistry {
    metas: HashMap<String, SkillMeta>,
    skill_md: HashMap<String, String>,
    references: HashMap<(String, String), LoadedReference>,
    assets: HashMap<(String, String), LoadedAsset>,
    scripts: HashMap<(String, String), ScriptResult>,
    warnings: Vec<SkillRegistryWarning>,
}

impl InMemorySkillRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert_skill(&mut self, meta: SkillMeta, skill_md: impl Into<String>) {
        self.skill_md.insert(meta.id.clone(), skill_md.into());
        self.metas.insert(meta.id.clone(), meta);
    }

    pub fn insert_reference(&mut self, reference: LoadedReference) {
        self.references
            .insert((reference.skill.clone(), reference.path.clone()), reference);
    }

    pub fn insert_script_result(&mut self, result: ScriptResult) {
        self.scripts
            .insert((result.skill.clone(), result.script.clone()), result);
    }

    pub fn insert_asset(&mut self, asset: LoadedAsset) {
        self.assets
            .insert((asset.skill.clone(), asset.path.clone()), asset);
    }

    pub fn push_warning(&mut self, warning: SkillRegistryWarning) {
        self.warnings.push(warning);
    }
}

#[async_trait]
impl SkillRegistry for InMemorySkillRegistry {
    fn list(&self) -> Vec<SkillMeta> {
        let mut out: Vec<SkillMeta> = self.metas.values().cloned().collect();
        out.sort_by(|a, b| a.id.cmp(&b.id));
        out
    }

    fn warnings(&self) -> Vec<SkillRegistryWarning> {
        self.warnings.clone()
    }

    fn get(&self, skill_id: &str) -> Option<SkillMeta> {
        self.metas.get(skill_id).cloned()
    }

    async fn read_skill_md(&self, skill_id: &str) -> Result<String, SkillRegistryError> {
        self.skill_md
            .get(skill_id)
            .cloned()
            .ok_or_else(|| SkillRegistryError::UnknownSkill(skill_id.to_string()))
    }

    async fn load_resource(
        &self,
        skill_id: &str,
        kind: SkillResourceKind,
        path: &str,
    ) -> Result<SkillResource, SkillRegistryError> {
        match kind {
            SkillResourceKind::Reference => self
                .references
                .get(&(skill_id.to_string(), path.to_string()))
                .cloned()
                .map(SkillResource::Reference)
                .ok_or_else(|| {
                    SkillRegistryError::Unsupported(format!("reference not available: {path}"))
                }),
            SkillResourceKind::Asset => self
                .assets
                .get(&(skill_id.to_string(), path.to_string()))
                .cloned()
                .map(SkillResource::Asset)
                .ok_or_else(|| {
                    SkillRegistryError::Unsupported(format!("asset not available: {path}"))
                }),
        }
    }

    async fn run_script(
        &self,
        skill_id: &str,
        script: &str,
        _args: &[String],
    ) -> Result<ScriptResult, SkillRegistryError> {
        self.scripts
            .get(&(skill_id.to_string(), script.to_string()))
            .cloned()
            .ok_or_else(|| {
                SkillRegistryError::Unsupported(format!("script not available: {script}"))
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn in_memory_registry_happy_path() {
        let mut reg = InMemorySkillRegistry::new();
        reg.insert_skill(
            SkillMeta {
                id: "s1".to_string(),
                name: "s1".to_string(),
                description: "d".to_string(),
                allowed_tools: vec!["read_file".to_string()],
            },
            "---\nname: s1\ndescription: d\n---\nBody\n",
        );

        let md = reg.read_skill_md("s1").await.unwrap();
        assert!(md.contains("Body"));

        reg.insert_reference(LoadedReference {
            skill: "s1".to_string(),
            path: "references/a.md".to_string(),
            sha256: "x".to_string(),
            truncated: false,
            content: "hello".to_string(),
            bytes: 5,
        });
        let r = reg
            .load_resource("s1", SkillResourceKind::Reference, "references/a.md")
            .await
            .unwrap();
        let SkillResource::Reference(r) = r else {
            panic!("expected reference resource");
        };
        assert_eq!(r.content, "hello");

        reg.insert_script_result(ScriptResult {
            skill: "s1".to_string(),
            script: "scripts/a.sh".to_string(),
            sha256: "x".to_string(),
            truncated_stdout: false,
            truncated_stderr: false,
            exit_code: 0,
            stdout: "ok".to_string(),
            stderr: "".to_string(),
        });
        let s = reg.run_script("s1", "scripts/a.sh", &[]).await.unwrap();
        assert_eq!(s.exit_code, 0);

        reg.insert_asset(LoadedAsset {
            skill: "s1".to_string(),
            path: "assets/logo.png".to_string(),
            sha256: "x".to_string(),
            truncated: false,
            bytes: 4,
            media_type: Some("image/png".to_string()),
            encoding: "base64".to_string(),
            content: "iVBORw==".to_string(),
        });
        let a = reg
            .load_resource("s1", SkillResourceKind::Asset, "assets/logo.png")
            .await
            .unwrap();
        let SkillResource::Asset(a) = a else {
            panic!("expected asset resource");
        };
        assert_eq!(a.media_type.as_deref(), Some("image/png"));

        let _ = json!({"ok": true});
    }
}
