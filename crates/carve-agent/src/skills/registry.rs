use crate::skills::skill_md::{parse_skill_md, SkillFrontmatter};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

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
    /// Skill root directory (contains `SKILL.md`, `references/`, `scripts/`, ...).
    #[serde(skip)]
    pub root_dir: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkillRegistryWarning {
    pub path: PathBuf,
    pub reason: String,
}

#[derive(Debug, Clone, Default)]
struct Index {
    metas: Vec<SkillMeta>,
    by_id: HashMap<String, SkillMeta>,
    warnings: Vec<SkillRegistryWarning>,
}

/// A registry for discovering skills on disk.
///
/// Discovery is metadata-only: it reads `SKILL.md` frontmatter and builds an index.
#[derive(Debug, Clone)]
pub struct SkillRegistry {
    roots: Vec<PathBuf>,
    index: Arc<RwLock<Option<Index>>>,
}

impl SkillRegistry {
    pub fn new(roots: Vec<PathBuf>) -> Self {
        Self {
            roots,
            index: Arc::new(RwLock::new(None)),
        }
    }

    pub fn from_root(root: impl Into<PathBuf>) -> Self {
        Self::new(vec![root.into()])
    }

    /// Return all discovered skills (metadata only).
    pub fn list(&self) -> Vec<SkillMeta> {
        self.ensure_indexed();
        self.index
            .read()
            .unwrap()
            .as_ref()
            .map(|idx| idx.metas.clone())
            .unwrap_or_default()
    }

    /// Diagnostics for skills that were skipped due to spec violations or IO errors.
    pub fn warnings(&self) -> Vec<SkillRegistryWarning> {
        self.ensure_indexed();
        self.index
            .read()
            .unwrap()
            .as_ref()
            .map(|idx| idx.warnings.clone())
            .unwrap_or_default()
    }

    /// Find skill metadata by id.
    pub fn get(&self, skill_id: &str) -> Option<SkillMeta> {
        self.ensure_indexed();
        self.index
            .read()
            .unwrap()
            .as_ref()
            .and_then(|idx| idx.by_id.get(skill_id).cloned())
    }

    /// Resolve a user-supplied skill key.
    ///
    /// Accepts:
    /// - exact id match
    pub fn resolve(&self, key: &str) -> Option<SkillMeta> {
        let key = key.trim();
        if let Some(meta) = self.get(key) {
            return Some(meta);
        }
        None
    }

    pub fn skill_md_path(&self, skill: &SkillMeta) -> PathBuf {
        skill.root_dir.join("SKILL.md")
    }

    pub fn refresh(&self) {
        *self.index.write().unwrap() = None;
    }

    fn ensure_indexed(&self) {
        if self.index.read().unwrap().is_some() {
            return;
        }

        let mut metas = Vec::new();
        let mut warnings = Vec::new();
        for root in &self.roots {
            let (m, w) = discover_under_root(root);
            metas.extend(m);
            warnings.extend(w);
        }

        metas.sort_by(|a, b| a.id.cmp(&b.id));
        let mut by_id = HashMap::new();
        for m in &metas {
            by_id.insert(m.id.clone(), m.clone());
        }

        warnings.sort_by(|a, b| a.path.cmp(&b.path));
        *self.index.write().unwrap() = Some(Index {
            metas,
            by_id,
            warnings,
        });
    }
}

fn discover_under_root(root: &Path) -> (Vec<SkillMeta>, Vec<SkillRegistryWarning>) {
    let mut metas = Vec::new();
    let mut warnings = Vec::new();

    let root = match fs::canonicalize(root) {
        Ok(p) => p,
        Err(e) => {
            warnings.push(SkillRegistryWarning {
                path: root.to_path_buf(),
                reason: format!("failed to access skills root: {e}"),
            });
            return (metas, warnings);
        }
    };

    let entries = match fs::read_dir(&root) {
        Ok(e) => e,
        Err(e) => {
            warnings.push(SkillRegistryWarning {
                path: root,
                reason: format!("failed to read skills root: {e}"),
            });
            return (metas, warnings);
        }
    };

    for entry in entries.flatten() {
        let path = entry.path();
        let ft = match entry.file_type() {
            Ok(ft) => ft,
            Err(_) => continue,
        };
        if !ft.is_dir() {
            continue;
        }
        let dir_name = match path.file_name().and_then(|s| s.to_str()) {
            Some(s) => s.to_string(),
            None => continue,
        };
        if dir_name.starts_with('.') {
            continue;
        }
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
            Ok(meta) => metas.push(meta),
            Err(reason) => warnings.push(SkillRegistryWarning {
                path: skill_md,
                reason,
            }),
        }
    }

    (metas, warnings)
}

fn meta_from_skill_md_path(
    dir_name: &str,
    skill_md: &Path,
) -> Result<SkillMeta, String> {
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

    Ok(SkillMeta {
        id: name.clone(),
        name,
        description,
        allowed_tools,
        root_dir,
    })
}

fn validate_dir_name(dir_name: &str) -> Result<(), String> {
    // Name validation is enforced by `parse_skill_md` too, but we validate directory
    // names early to produce clearer diagnostics (and to avoid reading SKILL.md).
    // See agentskills spec: lowercase kebab-case and length limit.
    if dir_name.chars().count() > 64 {
        return Err("directory name must be <= 64 characters".to_string());
    }
    let mut prev_hyphen = false;
    for (i, ch) in dir_name.chars().enumerate() {
        let is_hyphen = ch == '-';
        let is_ok = ch.is_ascii_lowercase() || ch.is_ascii_digit() || is_hyphen;
        if !is_ok {
            return Err("directory name must be lowercase kebab-case ([a-z0-9-])".to_string());
        }
        if i == 0 && is_hyphen {
            return Err("directory name must not start with '-'".to_string());
        }
        if prev_hyphen && is_hyphen {
            return Err("directory name must not contain consecutive '-'".to_string());
        }
        prev_hyphen = is_hyphen;
    }
    if dir_name.ends_with('-') {
        return Err("directory name must not end with '-'".to_string());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::TempDir;

    #[test]
    fn registry_discovers_skills_and_parses_frontmatter() {
        let td = TempDir::new().unwrap();
        let skills_root = td.path().join("skills");
        fs::create_dir_all(skills_root.join("docx-processing")).unwrap();
        let mut f = fs::File::create(skills_root.join("docx-processing").join("SKILL.md")).unwrap();
        writeln!(
            f,
            "{}",
            r#"---
name: docx-processing
description: Docs
allowed-tools: read_file
---
Body"#
        )
        .unwrap();

        let reg = SkillRegistry::from_root(&skills_root);
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

        let reg = SkillRegistry::from_root(&skills_root);
        assert_eq!(reg.list().len(), 1);
        assert_eq!(reg.list()[0].id, "good-skill");
        let warnings = reg.warnings();
        assert!(!warnings.is_empty());
        assert!(warnings.iter().any(|w| w.reason.contains("directory name")));
        assert!(warnings.iter().any(|w| w.reason.contains("missing SKILL.md")));
    }
}
