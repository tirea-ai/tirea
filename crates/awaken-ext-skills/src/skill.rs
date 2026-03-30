use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;

use crate::error::SkillError;
use crate::skill_md::parse_skill_md;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkillMeta {
    pub id: String,
    pub name: String,
    pub description: String,
    pub allowed_tools: Vec<String>,
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", content = "resource", rename_all = "snake_case")]
pub enum SkillResource {
    Reference(LoadedReference),
    Asset(LoadedAsset),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkillActivation {
    pub instructions: String,
}

/// A single skill with its own IO capabilities.
///
/// Each implementation encapsulates how to read instructions, load resources,
/// and run scripts.
#[async_trait]
pub trait Skill: Send + Sync + std::fmt::Debug {
    /// Metadata for this skill (id, name, description, allowed_tools).
    fn meta(&self) -> &SkillMeta;

    /// Read the raw SKILL.md content.
    async fn read_instructions(&self) -> Result<String, SkillError>;

    /// Resolve activation instructions for this skill.
    ///
    /// By default this parses the `SKILL.md` body and ignores activation args.
    async fn activate(&self, _args: Option<&Value>) -> Result<SkillActivation, SkillError> {
        let raw = self.read_instructions().await?;
        let doc = parse_skill_md(&raw).map_err(|e| SkillError::InvalidSkillMd(e.to_string()))?;
        Ok(SkillActivation {
            instructions: doc.body,
        })
    }

    /// Load a resource (reference or asset) by relative path.
    async fn load_resource(
        &self,
        kind: SkillResourceKind,
        path: &str,
    ) -> Result<SkillResource, SkillError>;

    /// Run a script by relative path with arguments.
    async fn run_script(&self, script: &str, args: &[String]) -> Result<ScriptResult, SkillError>;
}

/// Build a stable map key for skill materials.
pub fn material_key(skill_id: &str, relative_path: &str) -> String {
    format!("{skill_id}:{relative_path}")
}

/// Collect skills into a map, failing on duplicate IDs.
pub fn collect_skills(
    skills: Vec<Arc<dyn Skill>>,
) -> Result<HashMap<String, Arc<dyn Skill>>, SkillError> {
    let mut map: HashMap<String, Arc<dyn Skill>> = HashMap::new();
    for skill in skills {
        let id = skill.meta().id.clone();
        if map.contains_key(&id) {
            return Err(SkillError::DuplicateSkillId(id));
        }
        map.insert(id, skill);
    }
    Ok(map)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collect_skills_rejects_duplicates() {
        #[derive(Debug)]
        struct MockSkill(SkillMeta);

        #[async_trait]
        impl Skill for MockSkill {
            fn meta(&self) -> &SkillMeta {
                &self.0
            }
            async fn read_instructions(&self) -> Result<String, SkillError> {
                Ok(String::new())
            }
            async fn load_resource(
                &self,
                _kind: SkillResourceKind,
                _path: &str,
            ) -> Result<SkillResource, SkillError> {
                Err(SkillError::Unsupported("mock".into()))
            }
            async fn run_script(
                &self,
                _script: &str,
                _args: &[String],
            ) -> Result<ScriptResult, SkillError> {
                Err(SkillError::Unsupported("mock".into()))
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

        let skills: Vec<Arc<dyn Skill>> = vec![
            Arc::new(MockSkill(meta("a"))),
            Arc::new(MockSkill(meta("a"))),
        ];
        let err = collect_skills(skills).unwrap_err();
        assert!(matches!(err, SkillError::DuplicateSkillId(ref id) if id == "a"));
    }

    #[test]
    fn collect_skills_succeeds_for_unique_ids() {
        #[derive(Debug)]
        struct MockSkill(SkillMeta);

        #[async_trait]
        impl Skill for MockSkill {
            fn meta(&self) -> &SkillMeta {
                &self.0
            }
            async fn read_instructions(&self) -> Result<String, SkillError> {
                Ok(String::new())
            }
            async fn load_resource(
                &self,
                _kind: SkillResourceKind,
                _path: &str,
            ) -> Result<SkillResource, SkillError> {
                Err(SkillError::Unsupported("mock".into()))
            }
            async fn run_script(
                &self,
                _script: &str,
                _args: &[String],
            ) -> Result<ScriptResult, SkillError> {
                Err(SkillError::Unsupported("mock".into()))
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

        let skills: Vec<Arc<dyn Skill>> = vec![
            Arc::new(MockSkill(meta("alpha"))),
            Arc::new(MockSkill(meta("beta"))),
        ];
        let map = collect_skills(skills).unwrap();
        assert_eq!(map.len(), 2);
        assert!(map.contains_key("alpha"));
        assert!(map.contains_key("beta"));
    }

    #[test]
    fn material_key_format() {
        assert_eq!(
            material_key("my-skill", "references/guide.md"),
            "my-skill:references/guide.md"
        );
        assert_eq!(material_key("", "path"), ":path");
    }

    #[test]
    fn skill_meta_serde_roundtrip() {
        let meta = SkillMeta {
            id: "test".into(),
            name: "test".into(),
            description: "A test".into(),
            allowed_tools: vec!["Bash".into(), "Read".into()],
        };
        let json = serde_json::to_value(&meta).unwrap();
        let parsed: SkillMeta = serde_json::from_value(json).unwrap();
        assert_eq!(parsed, meta);
    }

    #[test]
    fn skill_activation_serde_roundtrip() {
        let activation = SkillActivation {
            instructions: "Do something.".into(),
        };
        let json = serde_json::to_value(&activation).unwrap();
        let parsed: SkillActivation = serde_json::from_value(json).unwrap();
        assert_eq!(parsed, activation);
    }

    #[test]
    fn skill_resource_kind_as_str() {
        assert_eq!(SkillResourceKind::Reference.as_str(), "reference");
        assert_eq!(SkillResourceKind::Asset.as_str(), "asset");
    }

    #[test]
    fn skill_resource_kind_serde_roundtrip() {
        let kind = SkillResourceKind::Asset;
        let json = serde_json::to_value(kind).unwrap();
        assert_eq!(json.as_str(), Some("asset"));
        let parsed: SkillResourceKind = serde_json::from_value(json).unwrap();
        assert_eq!(parsed, kind);
    }

    #[test]
    fn skill_resource_reference_variant_serde() {
        let r = SkillResource::Reference(LoadedReference {
            skill: "s1".into(),
            path: "references/a.md".into(),
            sha256: "abc".into(),
            truncated: false,
            content: "hello".into(),
            bytes: 5,
        });
        let json = serde_json::to_value(&r).unwrap();
        assert_eq!(json["kind"], "reference");
        let parsed: SkillResource = serde_json::from_value(json).unwrap();
        assert_eq!(parsed, r);
    }

    #[test]
    fn skill_resource_asset_variant_serde() {
        let a = SkillResource::Asset(LoadedAsset {
            skill: "s1".into(),
            path: "assets/img.png".into(),
            sha256: "def".into(),
            truncated: false,
            bytes: 100,
            media_type: Some("image/png".into()),
            encoding: "base64".into(),
            content: "aGVsbG8=".into(),
        });
        let json = serde_json::to_value(&a).unwrap();
        assert_eq!(json["kind"], "asset");
        let parsed: SkillResource = serde_json::from_value(json).unwrap();
        assert_eq!(parsed, a);
    }

    #[test]
    fn loaded_reference_fields() {
        let r = LoadedReference {
            skill: "s".into(),
            path: "p".into(),
            sha256: "h".into(),
            truncated: true,
            content: "c".into(),
            bytes: 1,
        };
        assert!(r.truncated);
        assert_eq!(r.bytes, 1);
    }

    #[test]
    fn script_result_fields() {
        let s = ScriptResult {
            skill: "s".into(),
            script: "scripts/run.sh".into(),
            sha256: "abc".into(),
            truncated_stdout: false,
            truncated_stderr: true,
            exit_code: 1,
            stdout: "out".into(),
            stderr: "err".into(),
        };
        assert_eq!(s.exit_code, 1);
        assert!(s.truncated_stderr);
    }

    #[tokio::test]
    async fn default_skill_activation_uses_skill_md_body() {
        #[derive(Debug)]
        struct MockSkill {
            meta: SkillMeta,
            skill_md: String,
        }

        #[async_trait]
        impl Skill for MockSkill {
            fn meta(&self) -> &SkillMeta {
                &self.meta
            }

            async fn read_instructions(&self) -> Result<String, SkillError> {
                Ok(self.skill_md.clone())
            }

            async fn load_resource(
                &self,
                _kind: SkillResourceKind,
                _path: &str,
            ) -> Result<SkillResource, SkillError> {
                Err(SkillError::Unsupported("mock".into()))
            }

            async fn run_script(
                &self,
                _script: &str,
                _args: &[String],
            ) -> Result<ScriptResult, SkillError> {
                Err(SkillError::Unsupported("mock".into()))
            }
        }

        let skill = MockSkill {
            meta: SkillMeta {
                id: "demo".to_string(),
                name: "demo".to_string(),
                description: "demo".to_string(),
                allowed_tools: Vec::new(),
            },
            skill_md: "---\nname: demo\ndescription: Demo\n---\nUse the demo flow.\n".to_string(),
        };

        let activation = skill.activate(None).await.unwrap();
        assert_eq!(activation.instructions, "Use the demo flow.\n");
    }
}
