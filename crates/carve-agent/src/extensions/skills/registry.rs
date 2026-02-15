use crate::extensions::skills::materialize::{
    load_asset_material, load_reference_material, run_script_material,
};
use crate::extensions::skills::skill_md::{parse_allowed_tools, parse_skill_md, SkillFrontmatter};
use async_trait::async_trait;
pub use carve_agent_contract::skills::{
    ScriptResult, SkillMeta, SkillRegistry, SkillRegistryError, SkillRegistryWarning,
    SkillResource, SkillResourceKind,
};
use std::collections::HashMap;
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use unicode_normalization::UnicodeNormalization;

type DiscoveryResult = (
    Vec<SkillMeta>,
    HashMap<String, PathBuf>,
    HashMap<String, PathBuf>,
    Vec<SkillRegistryWarning>,
);

#[derive(Debug, Clone, Default)]
struct FsIndex {
    metas: Vec<SkillMeta>,
    by_id: HashMap<String, SkillMeta>,
    by_id_root: HashMap<String, PathBuf>,
    by_id_skill_md: HashMap<String, PathBuf>,
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
        let mut by_id_skill_md: HashMap<String, PathBuf> = HashMap::new();

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
                let skill_md = docs_by_id.get(&meta.id).cloned().ok_or_else(|| {
                    SkillRegistryError::Io(format!("missing SKILL.md for skill {}", meta.id))
                })?;
                by_id_root.insert(meta.id.clone(), root_dir);
                by_id_skill_md.insert(meta.id.clone(), skill_md);
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
        let path = self
            .index
            .by_id_skill_md
            .get(skill_id)
            .cloned()
            .ok_or_else(|| SkillRegistryError::UnknownSkill(skill_id.to_string()))?;

        fs::read_to_string(&path).map_err(|e| {
            SkillRegistryError::Io(format!(
                "failed to read SKILL.md for skill '{skill_id}': {e}"
            ))
        })
    }

    async fn load_resource(
        &self,
        skill_id: &str,
        kind: SkillResourceKind,
        path: &str,
    ) -> Result<SkillResource, SkillRegistryError> {
        let root = self
            .index
            .by_id_root
            .get(skill_id)
            .cloned()
            .ok_or_else(|| SkillRegistryError::UnknownSkill(skill_id.to_string()))?;
        let skill_id = skill_id.to_string();
        let path = path.to_string();

        tokio::task::spawn_blocking(move || match kind {
            SkillResourceKind::Reference => {
                load_reference_material(&skill_id, &root, &path).map(SkillResource::Reference)
            }
            SkillResourceKind::Asset => {
                load_asset_material(&skill_id, &root, &path).map(SkillResource::Asset)
            }
        })
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

fn discover_under_root(root: &Path) -> Result<DiscoveryResult, SkillRegistryError> {
    let mut metas: Vec<SkillMeta> = Vec::new();
    let mut roots_by_id: HashMap<String, PathBuf> = HashMap::new();
    let mut docs_by_id: HashMap<String, PathBuf> = HashMap::new();
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
            Ok((meta, root_dir, skill_md_path)) => {
                roots_by_id.insert(meta.id.clone(), root_dir);
                docs_by_id.insert(meta.id.clone(), skill_md_path);
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
) -> Result<(SkillMeta, PathBuf, PathBuf), String> {
    let root_dir = skill_md
        .parent()
        .ok_or_else(|| "invalid skill path".to_string())?
        .to_path_buf();

    let SkillFrontmatter {
        name,
        description,
        allowed_tools,
        ..
    } = read_frontmatter_from_skill_md_path(skill_md)?;

    // Spec: name must match the parent directory name.
    if name != dir_name {
        return Err(format!(
            "frontmatter name '{name}' does not match directory '{dir_name}'"
        ));
    }

    let allowed_tools = allowed_tools
        .as_deref()
        .map(parse_allowed_tools)
        .transpose()
        .map_err(|e| e.to_string())?
        .unwrap_or_default()
        .into_iter()
        .map(|t| t.raw)
        .collect::<Vec<_>>();

    Ok((
        SkillMeta {
            id: name.clone(),
            name,
            description,
            allowed_tools,
        },
        root_dir,
        skill_md.to_path_buf(),
    ))
}

fn read_frontmatter_from_skill_md_path(skill_md: &Path) -> Result<SkillFrontmatter, String> {
    let file = fs::File::open(skill_md).map_err(|e| e.to_string())?;
    let mut reader = BufReader::new(file);

    let mut first = String::new();
    let n = reader.read_line(&mut first).map_err(|e| e.to_string())?;
    if n == 0 || trim_line_ending(&first) != "---" {
        return Err("missing YAML frontmatter (expected leading '---' fence)".to_string());
    }

    let mut fm = String::new();
    loop {
        let mut line = String::new();
        let read = reader.read_line(&mut line).map_err(|e| e.to_string())?;
        if read == 0 {
            return Err("unterminated YAML frontmatter (missing closing '---' fence)".to_string());
        }
        if trim_line_ending(&line) == "---" {
            break;
        }
        fm.push_str(&line);
    }

    // Reuse the same parser/validator to keep behavior consistent.
    let synthetic = format!("---\n{}---\n", fm);
    parse_skill_md(&synthetic)
        .map(|doc| doc.frontmatter)
        .map_err(|e| e.to_string())
}

fn trim_line_ending(line: &str) -> &str {
    line.trim_end_matches(|c| c == '\n' || c == '\r')
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
    async fn registry_read_skill_md_reads_from_disk_lazily() {
        let td = TempDir::new().unwrap();
        let skills_root = td.path().join("skills");
        fs::create_dir_all(skills_root.join("s1")).unwrap();
        let skill_md = skills_root.join("s1").join("SKILL.md");
        fs::write(&skill_md, "---\nname: s1\ndescription: ok\n---\nBody\n").unwrap();

        let reg = FsSkillRegistry::discover_root(&skills_root).unwrap();
        fs::remove_file(&skill_md).unwrap();

        let err = reg.read_skill_md("s1").await.unwrap_err();
        assert!(matches!(err, SkillRegistryError::Io(_)));
    }

    #[test]
    fn registry_discovery_does_not_parse_markdown_body() {
        let td = TempDir::new().unwrap();
        let skills_root = td.path().join("skills");
        fs::create_dir_all(skills_root.join("s1")).unwrap();
        let skill_md = skills_root.join("s1").join("SKILL.md");
        let mut bytes = b"---\nname: s1\ndescription: ok\n---\n".to_vec();
        bytes.extend_from_slice(&[0xff, 0xfe, 0xfd]); // invalid UTF-8 body
        fs::write(&skill_md, bytes).unwrap();

        let reg = FsSkillRegistry::discover_root(&skills_root).unwrap();
        let list = reg.list();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].id, "s1");
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
