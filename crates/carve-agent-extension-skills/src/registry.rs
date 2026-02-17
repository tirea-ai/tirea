use crate::materialize::{load_asset_material, load_reference_material, run_script_material};
use crate::skill_md::{parse_allowed_tools, parse_skill_md, SkillFrontmatter};
use crate::{
    ScriptResult, Skill, SkillError, SkillMaterializeError, SkillMeta, SkillResource,
    SkillResourceKind, SkillWarning,
};
use async_trait::async_trait;
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use unicode_normalization::UnicodeNormalization;

/// A filesystem-backed skill.
///
/// Each `FsSkill` owns its root directory and SKILL.md path. Resource loading
/// and script execution are performed relative to `root_dir`.
///
/// Use [`FsSkill::discover`] to scan a directory for skills, or
/// [`FsSkill::discover_roots`] for multiple directories.
#[derive(Debug, Clone)]
pub struct FsSkill {
    meta: SkillMeta,
    root_dir: PathBuf,
    skill_md_path: PathBuf,
}

/// Result of a discovery scan: found skills and any warnings.
#[derive(Debug, Clone)]
pub struct DiscoveryResult {
    pub skills: Vec<FsSkill>,
    pub warnings: Vec<SkillWarning>,
}

impl FsSkill {
    /// Discover all valid skills under a single root directory.
    pub fn discover(root: impl Into<PathBuf>) -> Result<DiscoveryResult, SkillError> {
        Self::discover_roots(vec![root.into()])
    }

    /// Discover skills under multiple root directories.
    ///
    /// Returns an error if duplicate skill IDs are found across roots.
    pub fn discover_roots(roots: Vec<PathBuf>) -> Result<DiscoveryResult, SkillError> {
        let mut skills: Vec<FsSkill> = Vec::new();
        let mut warnings: Vec<SkillWarning> = Vec::new();
        let mut seen_ids: std::collections::HashSet<String> = std::collections::HashSet::new();

        for root in roots {
            let (found, w) = discover_under_root(&root)?;
            warnings.extend(w);

            for skill in found {
                if !seen_ids.insert(skill.meta.id.clone()) {
                    return Err(SkillError::DuplicateSkillId(skill.meta.id));
                }
                skills.push(skill);
            }
        }

        skills.sort_by(|a, b| a.meta.id.cmp(&b.meta.id));
        warnings.sort_by(|a, b| a.path.cmp(&b.path));

        Ok(DiscoveryResult { skills, warnings })
    }

    /// Collect discovered skills into a vec of trait objects.
    pub fn into_arc_skills(skills: Vec<FsSkill>) -> Vec<Arc<dyn Skill>> {
        skills.into_iter().map(|s| Arc::new(s) as Arc<dyn Skill>).collect()
    }
}

#[async_trait]
impl Skill for FsSkill {
    fn meta(&self) -> &SkillMeta {
        &self.meta
    }

    async fn read_instructions(&self) -> Result<String, SkillError> {
        fs::read_to_string(&self.skill_md_path).map_err(|e| {
            SkillError::Io(format!(
                "failed to read SKILL.md for skill '{}': {e}",
                self.meta.id
            ))
        })
    }

    async fn load_resource(
        &self,
        kind: SkillResourceKind,
        path: &str,
    ) -> Result<SkillResource, SkillError> {
        let root = self.root_dir.clone();
        let skill_id = self.meta.id.clone();
        let path = path.to_string();

        let materialized: Result<SkillResource, SkillMaterializeError> =
            tokio::task::spawn_blocking(move || match kind {
                SkillResourceKind::Reference => {
                    load_reference_material(&skill_id, &root, &path).map(SkillResource::Reference)
                }
                SkillResourceKind::Asset => {
                    load_asset_material(&skill_id, &root, &path).map(SkillResource::Asset)
                }
            })
            .await
            .map_err(|e| SkillError::Io(e.to_string()))?;

        materialized.map_err(SkillError::from)
    }

    async fn run_script(
        &self,
        script: &str,
        args: &[String],
    ) -> Result<ScriptResult, SkillError> {
        let result: Result<ScriptResult, SkillMaterializeError> =
            run_script_material(&self.meta.id, &self.root_dir, script, args).await;
        result.map_err(SkillError::from)
    }
}

fn discover_under_root(root: &Path) -> Result<(Vec<FsSkill>, Vec<SkillWarning>), SkillError> {
    let mut skills: Vec<FsSkill> = Vec::new();
    let mut warnings: Vec<SkillWarning> = Vec::new();

    let root = fs::canonicalize(root)
        .map_err(|e| SkillError::Io(format!("failed to access skills root: {e}")))?;

    let entries = fs::read_dir(&root)
        .map_err(|e| SkillError::Io(format!("failed to read skills root: {e}")))?;

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
            warnings.push(SkillWarning {
                path: path.clone(),
                reason,
            });
            continue;
        }

        let skill_md = path.join("SKILL.md");
        if !skill_md.is_file() {
            warnings.push(SkillWarning {
                path: path.clone(),
                reason: "missing SKILL.md".to_string(),
            });
            continue;
        }

        match build_fs_skill(&dir_name, &skill_md) {
            Ok(skill) => skills.push(skill),
            Err(reason) => warnings.push(SkillWarning {
                path: skill_md,
                reason,
            }),
        }
    }

    Ok((skills, warnings))
}

fn build_fs_skill(dir_name: &str, skill_md: &Path) -> Result<FsSkill, String> {
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

    Ok(FsSkill {
        meta: SkillMeta {
            id: name.clone(),
            name,
            description,
            allowed_tools,
        },
        root_dir,
        skill_md_path: skill_md.to_path_buf(),
    })
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
    fn discover_skills_and_parses_frontmatter() {
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

        let result = FsSkill::discover(&skills_root).unwrap();
        assert_eq!(result.skills.len(), 1);
        assert_eq!(result.skills[0].meta.id, "docx-processing");
        assert_eq!(result.skills[0].meta.name, "docx-processing");
        assert_eq!(
            result.skills[0].meta.allowed_tools,
            vec!["read_file".to_string()]
        );
    }

    #[test]
    fn discover_skips_invalid_skills_and_reports_warnings() {
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

        let result = FsSkill::discover(&skills_root).unwrap();
        assert_eq!(result.skills.len(), 1);
        assert_eq!(result.skills[0].meta.id, "good-skill");
        assert!(!result.warnings.is_empty());
        assert!(result
            .warnings
            .iter()
            .any(|w| w.reason.contains("directory name")));
        assert!(result
            .warnings
            .iter()
            .any(|w| w.reason.contains("missing SKILL.md")));
    }

    #[test]
    fn discover_skips_name_mismatch() {
        let td = TempDir::new().unwrap();
        let skills_root = td.path().join("skills");
        fs::create_dir_all(skills_root.join("good-skill")).unwrap();
        fs::write(
            skills_root.join("good-skill").join("SKILL.md"),
            "---\nname: other-skill\ndescription: ok\n---\nBody\n",
        )
        .unwrap();

        let result = FsSkill::discover(&skills_root).unwrap();
        assert!(result.skills.is_empty());
        assert!(result
            .warnings
            .iter()
            .any(|w| w.reason.contains("does not match directory")));
    }

    #[test]
    fn discover_does_not_recurse_into_nested_dirs() {
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

        let result = FsSkill::discover(&skills_root).unwrap();
        assert_eq!(result.skills.len(), 1);
        assert_eq!(result.skills[0].meta.id, "good-skill");
    }

    #[test]
    fn discover_skips_hidden_dirs_and_root_files() {
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

        let result = FsSkill::discover(&skills_root).unwrap();
        assert!(result.skills.is_empty());
    }

    #[test]
    fn discover_allows_i18n_directory_names() {
        let td = TempDir::new().unwrap();
        let skills_root = td.path().join("skills");
        fs::create_dir_all(skills_root.join("技能")).unwrap();
        fs::write(
            skills_root.join("技能").join("SKILL.md"),
            "---\nname: 技能\ndescription: ok\n---\nBody\n",
        )
        .unwrap();

        let result = FsSkill::discover(&skills_root).unwrap();
        assert_eq!(result.skills.len(), 1);
        assert_eq!(result.skills[0].meta.id, "技能");
    }

    #[tokio::test]
    async fn read_instructions_reads_from_disk_lazily() {
        let td = TempDir::new().unwrap();
        let skills_root = td.path().join("skills");
        fs::create_dir_all(skills_root.join("s1")).unwrap();
        let skill_md = skills_root.join("s1").join("SKILL.md");
        fs::write(&skill_md, "---\nname: s1\ndescription: ok\n---\nBody\n").unwrap();

        let result = FsSkill::discover(&skills_root).unwrap();
        let skill = &result.skills[0];
        fs::remove_file(&skill_md).unwrap();

        let err = skill.read_instructions().await.unwrap_err();
        assert!(matches!(err, SkillError::Io(_)));
    }

    #[test]
    fn discover_does_not_parse_markdown_body() {
        let td = TempDir::new().unwrap();
        let skills_root = td.path().join("skills");
        fs::create_dir_all(skills_root.join("s1")).unwrap();
        let skill_md = skills_root.join("s1").join("SKILL.md");
        let mut bytes = b"---\nname: s1\ndescription: ok\n---\n".to_vec();
        bytes.extend_from_slice(&[0xff, 0xfe, 0xfd]); // invalid UTF-8 body
        fs::write(&skill_md, bytes).unwrap();

        let result = FsSkill::discover(&skills_root).unwrap();
        assert_eq!(result.skills.len(), 1);
        assert_eq!(result.skills[0].meta.id, "s1");
    }

    #[test]
    fn discover_errors_for_missing_root() {
        let td = TempDir::new().unwrap();
        let missing = td.path().join("missing");
        let err = FsSkill::discover(&missing).unwrap_err();
        assert!(matches!(err, SkillError::Io(_)));
    }

    #[test]
    fn discover_rejects_duplicate_ids_across_roots() {
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

        let err = FsSkill::discover_roots(vec![root1, root2]).unwrap_err();
        assert!(matches!(err, SkillError::DuplicateSkillId(ref id) if id == "s1"));
    }

    #[test]
    fn normalize_relative_name_nfkc() {
        let s = "a\u{FF0D}b";
        let norm = normalize_name(s);
        assert!(!norm.is_empty());
    }

    #[test]
    fn validate_dir_name_rejects_parent_dir_like_segments() {
        assert!(validate_dir_name("a/b").is_err());
        assert!(validate_dir_name("..").is_err());
    }
}
