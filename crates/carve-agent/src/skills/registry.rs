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
    /// Human-facing name (from frontmatter `name` or fallback to `id`).
    pub name: String,
    /// Human-facing description (from frontmatter `description`).
    pub description: String,
    /// Tools suggested/allowed by this skill (optional).
    pub allowed_tools: Vec<String>,
    /// Skill root directory (contains `SKILL.md`, `references/`, `scripts/`, ...).
    #[serde(skip)]
    pub root_dir: PathBuf,
}

#[derive(Debug, Clone, Default)]
struct Index {
    metas: Vec<SkillMeta>,
    by_id: HashMap<String, SkillMeta>,
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
    /// - case-insensitive match on `name`
    pub fn resolve(&self, key: &str) -> Option<SkillMeta> {
        if let Some(meta) = self.get(key) {
            return Some(meta);
        }
        let key_lc = key.to_lowercase();
        self.list()
            .into_iter()
            .find(|m| m.name.to_lowercase() == key_lc)
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
        for root in &self.roots {
            metas.extend(discover_under_root(root));
        }

        metas.sort_by(|a, b| a.id.cmp(&b.id));
        let mut by_id = HashMap::new();
        for m in &metas {
            by_id.insert(m.id.clone(), m.clone());
        }

        *self.index.write().unwrap() = Some(Index { metas, by_id });
    }
}

fn discover_under_root(root: &Path) -> Vec<SkillMeta> {
    let mut out = Vec::new();
    let root = match fs::canonicalize(root) {
        Ok(p) => p,
        Err(_) => return out,
    };
    walk_for_skill_md(&root, &root, &mut out);
    out
}

fn walk_for_skill_md(base: &Path, dir: &Path, out: &mut Vec<SkillMeta>) {
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        let file_type = match entry.file_type() {
            Ok(ft) => ft,
            Err(_) => continue,
        };
        if file_type.is_dir() {
            // Avoid hidden dirs by convention.
            if path
                .file_name()
                .and_then(|s| s.to_str())
                .is_some_and(|s| s.starts_with('.'))
            {
                continue;
            }
            walk_for_skill_md(base, &path, out);
            continue;
        }

        if !file_type.is_file() {
            continue;
        }

        if path.file_name().and_then(|s| s.to_str()) != Some("SKILL.md") {
            continue;
        }

        if let Some(meta) = meta_from_skill_md_path(base, &path) {
            out.push(meta);
        }
    }
}

fn meta_from_skill_md_path(base: &Path, skill_md: &Path) -> Option<SkillMeta> {
    let root_dir = skill_md.parent()?.to_path_buf();
    let rel_dir = root_dir.strip_prefix(base).ok().unwrap_or(&root_dir);
    let id = rel_dir
        .components()
        .filter_map(|c| c.as_os_str().to_str())
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(":");

    let raw = fs::read_to_string(skill_md).ok()?;
    let doc = parse_skill_md(&raw);
    let SkillFrontmatter {
        name,
        description,
        allowed_tools,
    } = doc.frontmatter;

    let name = name.unwrap_or_else(|| id.clone());
    let description = description.unwrap_or_default();

    Some(SkillMeta {
        id,
        name,
        description,
        allowed_tools,
        root_dir,
    })
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
        fs::create_dir_all(skills_root.join("docx")).unwrap();
        let mut f = fs::File::create(skills_root.join("docx").join("SKILL.md")).unwrap();
        writeln!(
            f,
            "{}",
            r#"---
name: DOCX Processing
description: Docs
allowed-tools:
  - read_file
---
Body"#
        )
        .unwrap();

        let reg = SkillRegistry::from_root(&skills_root);
        let list = reg.list();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].id, "docx");
        assert_eq!(list[0].name, "DOCX Processing");
        assert_eq!(list[0].allowed_tools, vec!["read_file".to_string()]);
    }
}
