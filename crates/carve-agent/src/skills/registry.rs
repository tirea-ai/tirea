use crate::skills::materialize::{
    load_reference_material, run_script_material, SkillMaterializeError,
};
use crate::skills::skill_md::{parse_skill_md, SkillFrontmatter};
use crate::skills::state::{LoadedReference, ScriptResult};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use unicode_normalization::UnicodeNormalization;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkillMeta {
    /// Stable skill identifier (derived from the relative directory path).
    pub id: String,
    /// Skill name (spec): directory name and frontmatter `name`.
    pub name: String,
    /// Human-facing description (from frontmatter `description`, required by spec).
    pub description: String,
    /// Tools suggested/allowed by this skill (optional).
    pub allowed_tools: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkillRegistryWarning {
    pub path: PathBuf,
    pub reason: String,
}

#[derive(Debug, thiserror::Error)]
pub enum SkillRegistryError {
    #[error("unknown skill: {0}")]
    UnknownSkill(String),

    #[error("invalid SKILL.md: {0}")]
    InvalidSkillMd(String),

    #[error("materialize error: {0}")]
    Materialize(String),

    #[error("io error: {0}")]
    Io(String),

    #[error("duplicate skill id: {0}")]
    DuplicateSkillId(String),

    #[error("unsupported operation: {0}")]
    Unsupported(String),
}

impl From<SkillMaterializeError> for SkillRegistryError {
    fn from(e: SkillMaterializeError) -> Self {
        SkillRegistryError::Materialize(e.to_string())
    }
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

    async fn load_reference(
        &self,
        skill_id: &str,
        path: &str,
    ) -> Result<LoadedReference, SkillRegistryError>;

    async fn run_script(
        &self,
        skill_id: &str,
        script: &str,
        args: &[String],
    ) -> Result<ScriptResult, SkillRegistryError>;
}

#[derive(Debug, Clone, Default)]
struct FsIndex {
    metas: Vec<SkillMeta>,
    by_id: HashMap<String, SkillMeta>,
    by_id_root: HashMap<String, PathBuf>,
    by_id_skill_md: HashMap<String, String>,
    warnings: Vec<SkillRegistryWarning>,
}

/// A registry for skills discovered from the local filesystem.
///
/// Construction performs IO and builds an immutable in-memory index.
#[derive(Debug, Clone)]
pub struct FsSkillRegistry {
    index: FsIndex,
}

impl FsSkillRegistry {
    pub fn discover_root(root: impl Into<PathBuf>) -> Result<Self, SkillRegistryError> {
        Self::discover_roots(vec![root.into()])
    }

    pub fn discover_roots(roots: Vec<PathBuf>) -> Result<Self, SkillRegistryError> {
        let mut metas: Vec<SkillMeta> = Vec::new();
        let mut warnings: Vec<SkillRegistryWarning> = Vec::new();
        let mut by_id_root: HashMap<String, PathBuf> = HashMap::new();
        let mut by_id_skill_md: HashMap<String, String> = HashMap::new();

        for root in roots {
            let (m, roots_by_id, docs_by_id, w) = discover_under_root(&root)?;
            warnings.extend(w);

            for meta in m {
                if by_id_root.contains_key(&meta.id) {
                    return Err(SkillRegistryError::DuplicateSkillId(meta.id));
                }
                let root_dir = roots_by_id.get(&meta.id).cloned().ok_or_else(|| {
                    SkillRegistryError::Io(format!("missing root dir for skill {}", meta.id))
                })?;
                let raw = docs_by_id.get(&meta.id).cloned().ok_or_else(|| {
                    SkillRegistryError::Io(format!("missing SKILL.md for skill {}", meta.id))
                })?;
                by_id_root.insert(meta.id.clone(), root_dir);
                by_id_skill_md.insert(meta.id.clone(), raw);
                metas.push(meta);
            }
        }

        metas.sort_by(|a, b| a.id.cmp(&b.id));
        let mut by_id: HashMap<String, SkillMeta> = HashMap::new();
        for m in &metas {
            by_id.insert(m.id.clone(), m.clone());
        }
        warnings.sort_by(|a, b| a.path.cmp(&b.path));

        Ok(Self {
            index: FsIndex {
                metas,
                by_id,
                by_id_root,
                by_id_skill_md,
                warnings,
            },
        })
    }
}

#[async_trait]
impl SkillRegistry for FsSkillRegistry {
    fn list(&self) -> Vec<SkillMeta> {
        self.index.metas.clone()
    }

    fn warnings(&self) -> Vec<SkillRegistryWarning> {
        self.index.warnings.clone()
    }

    fn get(&self, skill_id: &str) -> Option<SkillMeta> {
        self.index.by_id.get(skill_id).cloned()
    }

    async fn read_skill_md(&self, skill_id: &str) -> Result<String, SkillRegistryError> {
        self.index
            .by_id_skill_md
            .get(skill_id)
            .cloned()
            .ok_or_else(|| SkillRegistryError::UnknownSkill(skill_id.to_string()))
    }

    async fn load_reference(
        &self,
        skill_id: &str,
        path: &str,
    ) -> Result<LoadedReference, SkillRegistryError> {
        let root = self
            .index
            .by_id_root
            .get(skill_id)
            .cloned()
            .ok_or_else(|| SkillRegistryError::UnknownSkill(skill_id.to_string()))?;
        let skill_id = skill_id.to_string();
        let path = path.to_string();

        tokio::task::spawn_blocking(move || load_reference_material(&skill_id, &root, &path))
            .await
            .map_err(|e| SkillRegistryError::Io(e.to_string()))?
            .map_err(SkillRegistryError::from)
    }

    async fn run_script(
        &self,
        skill_id: &str,
        script: &str,
        args: &[String],
    ) -> Result<ScriptResult, SkillRegistryError> {
        let root = self
            .index
            .by_id_root
            .get(skill_id)
            .cloned()
            .ok_or_else(|| SkillRegistryError::UnknownSkill(skill_id.to_string()))?;
        run_script_material(skill_id, &root, script, args)
            .await
            .map_err(SkillRegistryError::from)
    }
}

fn discover_under_root(
    root: &Path,
) -> Result<
    (
        Vec<SkillMeta>,
        HashMap<String, PathBuf>,
        HashMap<String, String>,
        Vec<SkillRegistryWarning>,
    ),
    SkillRegistryError,
> {
    let mut metas: Vec<SkillMeta> = Vec::new();
    let mut roots_by_id: HashMap<String, PathBuf> = HashMap::new();
    let mut docs_by_id: HashMap<String, String> = HashMap::new();
    let mut warnings: Vec<SkillRegistryWarning> = Vec::new();

    let root = fs::canonicalize(root)
        .map_err(|e| SkillRegistryError::Io(format!("failed to access skills root: {e}")))?;

    let entries = fs::read_dir(&root)
        .map_err(|e| SkillRegistryError::Io(format!("failed to read skills root: {e}")))?;

    for entry in entries.flatten() {
        let path = entry.path();
        let ft = match entry.file_type() {
            Ok(ft) => ft,
            Err(_) => continue,
        };
        if !ft.is_dir() {
            continue;
        }
        let dir_name_raw = match path.file_name().and_then(|s| s.to_str()) {
            Some(s) => s.to_string(),
            None => continue,
        };
        if dir_name_raw.starts_with('.') {
            continue;
        }

        let dir_name = normalize_name(&dir_name_raw);
        if let Err(reason) = validate_dir_name(&dir_name) {
            warnings.push(SkillRegistryWarning {
                path: path.clone(),
                reason,
            });
            continue;
        }

        let skill_md = path.join("SKILL.md");
        if !skill_md.is_file() {
            warnings.push(SkillRegistryWarning {
                path: path.clone(),
                reason: "missing SKILL.md".to_string(),
            });
            continue;
        }

        match meta_from_skill_md_path(&dir_name, &skill_md) {
            Ok((meta, root_dir, raw)) => {
                roots_by_id.insert(meta.id.clone(), root_dir);
                docs_by_id.insert(meta.id.clone(), raw);
                metas.push(meta);
            }
            Err(reason) => warnings.push(SkillRegistryWarning {
                path: skill_md,
                reason,
            }),
        }
    }

    Ok((metas, roots_by_id, docs_by_id, warnings))
}

fn meta_from_skill_md_path(
    dir_name: &str,
    skill_md: &Path,
) -> Result<(SkillMeta, PathBuf, String), String> {
    let root_dir = skill_md
        .parent()
        .ok_or_else(|| "invalid skill path".to_string())?
        .to_path_buf();

    let raw = fs::read_to_string(skill_md).map_err(|e| e.to_string())?;
    let doc = parse_skill_md(&raw).map_err(|e| e.to_string())?;
    let SkillFrontmatter {
        name,
        description,
        allowed_tools,
        ..
    } = doc.frontmatter;

    // Spec: name must match the parent directory name.
    if name != dir_name {
        return Err(format!(
            "frontmatter name '{name}' does not match directory '{dir_name}'"
        ));
    }

    let allowed_tools = allowed_tools
        .unwrap_or_default()
        .split_whitespace()
        .map(|s| s.to_string())
        .collect::<Vec<_>>();

    Ok((
        SkillMeta {
            id: name.clone(),
            name,
            description,
            allowed_tools,
        },
        root_dir,
        raw,
    ))
}

fn normalize_name(s: &str) -> String {
    s.trim().nfkc().collect::<String>()
}

fn validate_dir_name(dir_name: &str) -> Result<(), String> {
    // Name validation is enforced by `parse_skill_md` too, but we validate directory
    // names early to produce clearer diagnostics (and to avoid reading SKILL.md).
    // See agentskills spec: i18n letters/digits/hyphens, lowercase, length limit.
    if dir_name.is_empty() {
        return Err("directory name must be non-empty".to_string());
    }
    if dir_name.chars().count() > 64 {
        return Err("directory name must be <= 64 characters".to_string());
    }
    if dir_name != dir_name.to_lowercase() {
        return Err("directory name must be lowercase".to_string());
    }
    if dir_name.starts_with('-') {
        return Err("directory name must not start with '-'".to_string());
    }
    if dir_name.ends_with('-') {
        return Err("directory name must not end with '-'".to_string());
    }
    if dir_name.contains("--") {
        return Err("directory name must not contain consecutive '-'".to_string());
    }
    if !dir_name.chars().all(|c| c.is_alphanumeric() || c == '-') {
        return Err(
            "directory name contains invalid characters (only letters, digits, and '-' are allowed)"
                .to_string(),
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn registry_discovers_skills_and_parses_frontmatter() {
        let td = TempDir::new().unwrap();
        let skills_root = td.path().join("skills");
        fs::create_dir_all(skills_root.join("docx-processing")).unwrap();
        fs::write(
            skills_root.join("docx-processing").join("SKILL.md"),
            r#"---
name: docx-processing
description: Docs
allowed-tools: read_file
---
Body
"#,
        )
        .unwrap();

        let reg = FsSkillRegistry::discover_root(&skills_root).unwrap();
        let list = reg.list();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].id, "docx-processing");
        assert_eq!(list[0].name, "docx-processing");
        assert_eq!(list[0].allowed_tools, vec!["read_file".to_string()]);
    }

    #[test]
    fn registry_skips_invalid_skills_and_reports_warnings() {
        let td = TempDir::new().unwrap();
        let skills_root = td.path().join("skills");
        fs::create_dir_all(skills_root.join("good-skill")).unwrap();
        fs::create_dir_all(skills_root.join("BadSkill")).unwrap();
        fs::create_dir_all(skills_root.join("missing-skill-md")).unwrap();
        fs::write(
            skills_root.join("good-skill").join("SKILL.md"),
            "---\nname: good-skill\ndescription: ok\n---\nBody\n",
        )
        .unwrap();
        fs::write(
            skills_root.join("BadSkill").join("SKILL.md"),
            "---\nname: badskill\ndescription: x\n---\nBody\n",
        )
        .unwrap();

        let reg = FsSkillRegistry::discover_root(&skills_root).unwrap();
        assert_eq!(reg.list().len(), 1);
        assert_eq!(reg.list()[0].id, "good-skill");
        let warnings = reg.warnings();
        assert!(!warnings.is_empty());
        assert!(warnings.iter().any(|w| w.reason.contains("directory name")));
        assert!(warnings
            .iter()
            .any(|w| w.reason.contains("missing SKILL.md")));
    }

    #[test]
    fn registry_skips_name_mismatch() {
        let td = TempDir::new().unwrap();
        let skills_root = td.path().join("skills");
        fs::create_dir_all(skills_root.join("good-skill")).unwrap();
        fs::write(
            skills_root.join("good-skill").join("SKILL.md"),
            "---\nname: other-skill\ndescription: ok\n---\nBody\n",
        )
        .unwrap();

        let reg = FsSkillRegistry::discover_root(&skills_root).unwrap();
        assert!(reg.list().is_empty());
        let warnings = reg.warnings();
        assert!(warnings
            .iter()
            .any(|w| w.reason.contains("does not match directory")));
    }

    #[test]
    fn registry_does_not_recurse_into_nested_dirs() {
        let td = TempDir::new().unwrap();
        let skills_root = td.path().join("skills");
        fs::create_dir_all(skills_root.join("good-skill").join("nested")).unwrap();
        fs::write(
            skills_root.join("good-skill").join("SKILL.md"),
            "---\nname: good-skill\ndescription: ok\n---\nBody\n",
        )
        .unwrap();
        fs::write(
            skills_root
                .join("good-skill")
                .join("nested")
                .join("SKILL.md"),
            "---\nname: nested-skill\ndescription: ok\n---\nBody\n",
        )
        .unwrap();

        let reg = FsSkillRegistry::discover_root(&skills_root).unwrap();
        let list = reg.list();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].id, "good-skill");
        assert!(reg.get("nested-skill").is_none());
    }

    #[test]
    fn registry_skips_hidden_dirs_and_root_files() {
        let td = TempDir::new().unwrap();
        let skills_root = td.path().join("skills");
        fs::create_dir_all(skills_root.join(".hidden")).unwrap();
        fs::write(
            skills_root.join(".hidden").join("SKILL.md"),
            "---\nname: hidden\ndescription: ok\n---\nBody\n",
        )
        .unwrap();
        fs::write(
            skills_root.join("not-a-skill"),
            "---\nname: not-a-skill\ndescription: ok\n---\nBody\n",
        )
        .unwrap();

        let reg = FsSkillRegistry::discover_root(&skills_root).unwrap();
        assert!(reg.list().is_empty());
    }

    #[test]
    fn registry_allows_i18n_directory_names() {
        let td = TempDir::new().unwrap();
        let skills_root = td.path().join("skills");
        fs::create_dir_all(skills_root.join("技能")).unwrap();
        fs::write(
            skills_root.join("技能").join("SKILL.md"),
            "---\nname: 技能\ndescription: ok\n---\nBody\n",
        )
        .unwrap();

        let reg = FsSkillRegistry::discover_root(&skills_root).unwrap();
        let list = reg.list();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].id, "技能");
        assert_eq!(list[0].name, "技能");
    }

    #[tokio::test]
    async fn registry_read_skill_md_is_in_memory_after_discovery() {
        let td = TempDir::new().unwrap();
        let skills_root = td.path().join("skills");
        fs::create_dir_all(skills_root.join("s1")).unwrap();
        let skill_md = skills_root.join("s1").join("SKILL.md");
        fs::write(&skill_md, "---\nname: s1\ndescription: ok\n---\nBody\n").unwrap();

        let reg = FsSkillRegistry::discover_root(&skills_root).unwrap();
        fs::remove_file(&skill_md).unwrap();

        let raw = reg.read_skill_md("s1").await.unwrap();
        assert!(raw.contains("Body"));
    }

    #[test]
    fn registry_discovery_errors_for_missing_root() {
        let td = TempDir::new().unwrap();
        let missing = td.path().join("missing");
        let err = FsSkillRegistry::discover_root(&missing).unwrap_err();
        assert!(matches!(err, SkillRegistryError::Io(_)));
    }

    #[test]
    fn registry_discovery_rejects_duplicate_ids_across_roots() {
        let td = TempDir::new().unwrap();
        let root1 = td.path().join("skills1");
        let root2 = td.path().join("skills2");
        fs::create_dir_all(root1.join("s1")).unwrap();
        fs::create_dir_all(root2.join("s1")).unwrap();
        fs::write(
            root1.join("s1").join("SKILL.md"),
            "---\nname: s1\ndescription: ok\n---\nBody\n",
        )
        .unwrap();
        fs::write(
            root2.join("s1").join("SKILL.md"),
            "---\nname: s1\ndescription: ok\n---\nBody\n",
        )
        .unwrap();

        let err = FsSkillRegistry::discover_roots(vec![root1, root2]).unwrap_err();
        assert!(matches!(err, SkillRegistryError::DuplicateSkillId(ref id) if id == "s1"));
    }

    #[test]
    fn normalize_relative_name_nfkc() {
        // Full-width hyphen should normalize; exact behavior is implementation-defined but should be stable.
        let s = "a\u{FF0D}b";
        let norm = normalize_name(s);
        assert!(!norm.is_empty());
    }

    #[test]
    fn validate_dir_name_rejects_parent_dir_like_segments() {
        // Sanity: validate_dir_name doesn't allow slashes; this is enforced by the filesystem anyway.
        assert!(validate_dir_name("a/b").is_err());
        assert!(validate_dir_name("..").is_err());
    }
}
