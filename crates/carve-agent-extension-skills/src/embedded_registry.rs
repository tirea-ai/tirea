//! Embedded skill registry — skills loaded from in-memory content.
//!
//! This is the compile-time counterpart to [`FsSkillRegistry`]. Instead of
//! discovering skills from the filesystem, skills are provided as static string
//! slices (typically via `include_str!`). The registry parses and validates each
//! skill at construction time, so any errors are caught early.
//!
//! Scripts are **not** supported because there is no filesystem to execute from;
//! calling `run_script` returns [`SkillRegistryError::Unsupported`].
//!
//! # Example
//!
//! ```ignore
//! use carve_agent::extensions::skills::{EmbeddedSkill, EmbeddedSkillRegistry, SkillSubsystem};
//!
//! static SKILLS: &[EmbeddedSkill] = &[
//!     EmbeddedSkill {
//!         skill_md: include_str!("../skills/my-skill/SKILL.md"),
//!         references: &[
//!             ("references/guide.md", include_str!("../skills/my-skill/references/guide.md")),
//!         ],
//!     },
//! ];
//!
//! let registry = EmbeddedSkillRegistry::new(SKILLS).unwrap();
//! let subsystem = SkillSubsystem::new(std::sync::Arc::new(registry));
//! ```

use crate::resource_lookup::load_resource_from_maps;
use crate::skill_md::{parse_allowed_tools, parse_skill_md};
use crate::{
    LoadedAsset, LoadedReference, ScriptResult, SkillMeta, SkillRegistry, SkillRegistryError,
    SkillRegistryWarning, SkillResource, SkillResourceKind,
};
use async_trait::async_trait;
use base64::Engine as _;
use sha2::{Digest, Sha256};
use std::collections::HashMap;

/// A single skill with its content provided as static string slices.
///
/// Designed for use with `include_str!` to embed skill content at compile time.
pub struct EmbeddedSkill {
    /// Raw `SKILL.md` content (YAML frontmatter + markdown body).
    pub skill_md: &'static str,
    /// Reference files as `(relative_path, content)` pairs.
    ///
    /// Paths should match the convention used by filesystem skills,
    /// e.g. `"references/guide.md"`.
    pub references: &'static [(&'static str, &'static str)],
    /// Asset files as `(relative_path, content_base64, media_type)` tuples.
    pub assets: &'static [(&'static str, &'static str, Option<&'static str>)],
}

/// A skill registry built from in-memory content.
///
/// Unlike [`FsSkillRegistry`], this registry has no filesystem dependency at
/// runtime. All skills and references are parsed and validated at construction
/// time via [`EmbeddedSkillRegistry::new`].
#[derive(Debug, Clone)]
pub struct EmbeddedSkillRegistry {
    metas: Vec<SkillMeta>,
    by_id: HashMap<String, SkillMeta>,
    skill_md: HashMap<String, String>,
    references: HashMap<(String, String), LoadedReference>,
    assets: HashMap<(String, String), LoadedAsset>,
}

impl EmbeddedSkillRegistry {
    /// Build a registry from a slice of embedded skills.
    ///
    /// Each skill's `SKILL.md` is parsed and validated against the agentskills
    /// spec. Returns an error if any skill is invalid or if duplicate IDs are
    /// found.
    pub fn new(skills: &[EmbeddedSkill]) -> Result<Self, SkillRegistryError> {
        let mut metas = Vec::new();
        let mut by_id = HashMap::new();
        let mut skill_md_map = HashMap::new();
        let mut references = HashMap::new();
        let mut assets = HashMap::new();

        for embedded in skills {
            let doc = parse_skill_md(embedded.skill_md)
                .map_err(|e| SkillRegistryError::InvalidSkillMd(e.to_string()))?;

            let fm = &doc.frontmatter;
            let id = fm.name.clone();

            if by_id.contains_key(&id) {
                return Err(SkillRegistryError::DuplicateSkillId(id));
            }

            let allowed_tools = fm
                .allowed_tools
                .as_deref()
                .map(parse_allowed_tools)
                .transpose()
                .map_err(|e| SkillRegistryError::InvalidSkillMd(e.to_string()))?
                .unwrap_or_default()
                .into_iter()
                .map(|t| t.raw)
                .collect::<Vec<_>>();

            let meta = SkillMeta {
                id: id.clone(),
                name: id.clone(),
                description: fm.description.clone(),
                allowed_tools,
            };

            for &(path, content) in embedded.references {
                let sha = format!("{:x}", Sha256::digest(content.as_bytes()));
                references.insert(
                    (id.clone(), path.to_string()),
                    LoadedReference {
                        skill: id.clone(),
                        path: path.to_string(),
                        sha256: sha,
                        truncated: false,
                        content: content.to_string(),
                        bytes: content.len() as u64,
                    },
                );
            }

            for &(path, content_base64, media_type) in embedded.assets {
                let decoded = base64::engine::general_purpose::STANDARD
                    .decode(content_base64.as_bytes())
                    .map_err(|e| {
                        SkillRegistryError::InvalidSkillMd(format!(
                            "invalid base64 asset '{path}' for skill '{id}': {e}"
                        ))
                    })?;
                let sha = format!("{:x}", Sha256::digest(&decoded));
                assets.insert(
                    (id.clone(), path.to_string()),
                    LoadedAsset {
                        skill: id.clone(),
                        path: path.to_string(),
                        sha256: sha,
                        truncated: false,
                        bytes: decoded.len() as u64,
                        media_type: media_type.map(|m| m.to_string()),
                        encoding: "base64".to_string(),
                        content: content_base64.to_string(),
                    },
                );
            }

            skill_md_map.insert(id.clone(), embedded.skill_md.to_string());
            by_id.insert(id, meta.clone());
            metas.push(meta);
        }

        metas.sort_by(|a, b| a.id.cmp(&b.id));

        Ok(Self {
            metas,
            by_id,
            skill_md: skill_md_map,
            references,
            assets,
        })
    }
}

#[async_trait]
impl SkillRegistry for EmbeddedSkillRegistry {
    fn list(&self) -> Vec<SkillMeta> {
        self.metas.clone()
    }

    fn warnings(&self) -> Vec<SkillRegistryWarning> {
        Vec::new()
    }

    fn get(&self, skill_id: &str) -> Option<SkillMeta> {
        self.by_id.get(skill_id).cloned()
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
        load_resource_from_maps(&self.references, &self.assets, skill_id, kind, path)
    }

    async fn run_script(
        &self,
        _skill_id: &str,
        script: &str,
        _args: &[String],
    ) -> Result<ScriptResult, SkillRegistryError> {
        Err(SkillRegistryError::Unsupported(format!(
            "embedded skills do not support script execution: {script}"
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const VALID_SKILL_MD: &str = "\
---
name: test-skill
description: A test skill for unit testing.
allowed-tools: read_file write_file
---
# Test Skill

Follow these instructions to do testing.
";

    const VALID_SKILL_MD_2: &str = "\
---
name: another-skill
description: Another test skill.
---
# Another

More instructions.
";

    const REFERENCE_CONTENT: &str = "# Guide\n\nSome reference material.\n";

    #[test]
    fn new_parses_valid_skills() {
        let skills = &[EmbeddedSkill {
            skill_md: VALID_SKILL_MD,
            references: &[("references/guide.md", REFERENCE_CONTENT)],
            assets: &[],
        }];

        let reg = EmbeddedSkillRegistry::new(skills).unwrap();
        let list = reg.list();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].id, "test-skill");
        assert_eq!(list[0].name, "test-skill");
        assert_eq!(list[0].description, "A test skill for unit testing.");
        assert_eq!(
            list[0].allowed_tools,
            vec!["read_file".to_string(), "write_file".to_string()]
        );
    }

    #[test]
    fn new_rejects_invalid_skill_md() {
        let skills = &[EmbeddedSkill {
            skill_md: "not valid frontmatter",
            references: &[],
            assets: &[],
        }];

        let err = EmbeddedSkillRegistry::new(skills).unwrap_err();
        assert!(matches!(err, SkillRegistryError::InvalidSkillMd(_)));
    }

    #[test]
    fn new_rejects_duplicate_ids() {
        let skills = &[
            EmbeddedSkill {
                skill_md: VALID_SKILL_MD,
                references: &[],
                assets: &[],
            },
            EmbeddedSkill {
                skill_md: VALID_SKILL_MD,
                references: &[],
                assets: &[],
            },
        ];

        let err = EmbeddedSkillRegistry::new(skills).unwrap_err();
        assert!(matches!(err, SkillRegistryError::DuplicateSkillId(ref id) if id == "test-skill"));
    }

    #[test]
    fn new_sorts_by_id() {
        let skills = &[
            EmbeddedSkill {
                skill_md: VALID_SKILL_MD,
                references: &[],
                assets: &[],
            },
            EmbeddedSkill {
                skill_md: VALID_SKILL_MD_2,
                references: &[],
                assets: &[],
            },
        ];

        let reg = EmbeddedSkillRegistry::new(skills).unwrap();
        let list = reg.list();
        let ids: Vec<&str> = list.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(ids, vec!["another-skill", "test-skill"]);
    }

    #[test]
    fn get_returns_meta() {
        let skills = &[EmbeddedSkill {
            skill_md: VALID_SKILL_MD,
            references: &[],
            assets: &[],
        }];
        let reg = EmbeddedSkillRegistry::new(skills).unwrap();

        assert!(reg.get("test-skill").is_some());
        assert!(reg.get("nonexistent").is_none());
    }

    #[tokio::test]
    async fn read_skill_md_returns_raw_content() {
        let skills = &[EmbeddedSkill {
            skill_md: VALID_SKILL_MD,
            references: &[],
            assets: &[],
        }];
        let reg = EmbeddedSkillRegistry::new(skills).unwrap();

        let md = reg.read_skill_md("test-skill").await.unwrap();
        assert!(md.contains("# Test Skill"));
        assert!(md.contains("name: test-skill"));
    }

    #[tokio::test]
    async fn read_skill_md_unknown_returns_error() {
        let reg = EmbeddedSkillRegistry::new(&[]).unwrap();
        let err = reg.read_skill_md("nope").await.unwrap_err();
        assert!(matches!(err, SkillRegistryError::UnknownSkill(_)));
    }

    #[tokio::test]
    async fn load_reference_returns_content() {
        let skills = &[EmbeddedSkill {
            skill_md: VALID_SKILL_MD,
            references: &[("references/guide.md", REFERENCE_CONTENT)],
            assets: &[],
        }];
        let reg = EmbeddedSkillRegistry::new(skills).unwrap();

        let r = reg
            .load_resource(
                "test-skill",
                SkillResourceKind::Reference,
                "references/guide.md",
            )
            .await
            .unwrap();
        let SkillResource::Reference(r) = r else {
            panic!("expected reference resource");
        };
        assert_eq!(r.skill, "test-skill");
        assert_eq!(r.path, "references/guide.md");
        assert_eq!(r.content, REFERENCE_CONTENT);
        assert_eq!(r.bytes, REFERENCE_CONTENT.len() as u64);
        assert!(!r.truncated);
        assert!(!r.sha256.is_empty());
    }

    #[tokio::test]
    async fn load_reference_unknown_returns_error() {
        let skills = &[EmbeddedSkill {
            skill_md: VALID_SKILL_MD,
            references: &[],
            assets: &[],
        }];
        let reg = EmbeddedSkillRegistry::new(skills).unwrap();

        let err = reg
            .load_resource(
                "test-skill",
                SkillResourceKind::Reference,
                "references/missing.md",
            )
            .await
            .unwrap_err();
        assert!(matches!(err, SkillRegistryError::Unsupported(_)));
    }

    #[tokio::test]
    async fn run_script_returns_unsupported() {
        let skills = &[EmbeddedSkill {
            skill_md: VALID_SKILL_MD,
            references: &[],
            assets: &[],
        }];
        let reg = EmbeddedSkillRegistry::new(skills).unwrap();

        let err = reg
            .run_script("test-skill", "scripts/run.sh", &[])
            .await
            .unwrap_err();
        assert!(matches!(err, SkillRegistryError::Unsupported(_)));
        assert!(err.to_string().contains("embedded skills do not support"));
    }

    #[test]
    fn warnings_is_empty() {
        let reg = EmbeddedSkillRegistry::new(&[]).unwrap();
        assert!(reg.warnings().is_empty());
    }

    #[test]
    fn empty_skills_slice_produces_empty_registry() {
        let reg = EmbeddedSkillRegistry::new(&[]).unwrap();
        assert!(reg.list().is_empty());
        assert!(reg.get("anything").is_none());
    }

    #[test]
    fn skill_without_allowed_tools_has_empty_vec() {
        let skills = &[EmbeddedSkill {
            skill_md: VALID_SKILL_MD_2, // no allowed-tools field
            references: &[],
            assets: &[],
        }];
        let reg = EmbeddedSkillRegistry::new(skills).unwrap();
        let meta = reg.get("another-skill").unwrap();
        assert!(meta.allowed_tools.is_empty());
    }

    #[test]
    fn resolve_trims_whitespace_and_finds_skill() {
        let skills = &[EmbeddedSkill {
            skill_md: VALID_SKILL_MD,
            references: &[],
            assets: &[],
        }];
        let reg = EmbeddedSkillRegistry::new(skills).unwrap();

        assert!(reg.resolve("  test-skill  ").is_some());
        assert!(reg.resolve("nonexistent").is_none());
    }

    static MULTI_REF_SKILLS: &[EmbeddedSkill] = &[EmbeddedSkill {
        skill_md: VALID_SKILL_MD,
        references: &[
            ("references/a.md", "Content A"),
            ("references/b.md", "Content B"),
        ],
        assets: &[],
    }];

    #[tokio::test]
    async fn multiple_references_per_skill() {
        let reg = EmbeddedSkillRegistry::new(MULTI_REF_SKILLS).unwrap();

        let a = reg
            .load_resource(
                "test-skill",
                SkillResourceKind::Reference,
                "references/a.md",
            )
            .await
            .unwrap();
        let SkillResource::Reference(a) = a else {
            panic!("expected reference resource");
        };
        assert_eq!(a.content, "Content A");

        let b = reg
            .load_resource(
                "test-skill",
                SkillResourceKind::Reference,
                "references/b.md",
            )
            .await
            .unwrap();
        let SkillResource::Reference(b) = b else {
            panic!("expected reference resource");
        };
        assert_eq!(b.content, "Content B");
    }

    #[test]
    fn reference_sha256_is_deterministic() {
        let skills = &[EmbeddedSkill {
            skill_md: VALID_SKILL_MD,
            references: &[("references/guide.md", REFERENCE_CONTENT)],
            assets: &[],
        }];
        let reg1 = EmbeddedSkillRegistry::new(skills).unwrap();
        let reg2 = EmbeddedSkillRegistry::new(skills).unwrap();

        let hash1 = &reg1
            .references
            .get(&("test-skill".into(), "references/guide.md".into()))
            .unwrap()
            .sha256;
        let hash2 = &reg2
            .references
            .get(&("test-skill".into(), "references/guide.md".into()))
            .unwrap()
            .sha256;
        assert_eq!(hash1, hash2);
        // SHA-256 hex is 64 chars
        assert_eq!(hash1.len(), 64);
    }

    #[test]
    fn clone_produces_equal_registry() {
        let skills = &[EmbeddedSkill {
            skill_md: VALID_SKILL_MD,
            references: &[("references/guide.md", REFERENCE_CONTENT)],
            assets: &[],
        }];
        let reg = EmbeddedSkillRegistry::new(skills).unwrap();
        let cloned = reg.clone();

        assert_eq!(reg.list().len(), cloned.list().len());
        assert_eq!(reg.list()[0].id, cloned.list()[0].id);
    }

    #[tokio::test]
    async fn works_with_skill_subsystem() {
        use crate::SkillSubsystem;
        use std::sync::Arc;

        let skills = &[
            EmbeddedSkill {
                skill_md: VALID_SKILL_MD,
                references: &[("references/guide.md", REFERENCE_CONTENT)],
                assets: &[],
            },
            EmbeddedSkill {
                skill_md: VALID_SKILL_MD_2,
                references: &[],
                assets: &[],
            },
        ];
        let reg = EmbeddedSkillRegistry::new(skills).unwrap();
        let subsystem = SkillSubsystem::new(Arc::new(reg));

        // Subsystem should expose tools
        let tools = subsystem.tools();
        assert!(tools.contains_key("skill"));
        assert!(tools.contains_key("load_skill_resource"));
        assert!(tools.contains_key("skill_script"));

        // Registry should be accessible through subsystem
        let registry = subsystem.registry();
        assert_eq!(registry.list().len(), 2);

        let md = registry.read_skill_md("test-skill").await.unwrap();
        assert!(md.contains("# Test Skill"));
    }

    #[tokio::test]
    async fn load_reference_for_unknown_skill_returns_error() {
        let skills = &[EmbeddedSkill {
            skill_md: VALID_SKILL_MD,
            references: &[("references/guide.md", REFERENCE_CONTENT)],
            assets: &[],
        }];
        let reg = EmbeddedSkillRegistry::new(skills).unwrap();

        let err = reg
            .load_resource(
                "nonexistent-skill",
                SkillResourceKind::Reference,
                "references/guide.md",
            )
            .await
            .unwrap_err();
        assert!(matches!(
            err,
            SkillRegistryError::Unsupported(_) | SkillRegistryError::UnknownSkill(_)
        ));
    }
}
