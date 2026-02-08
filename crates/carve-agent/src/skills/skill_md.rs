use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use unicode_normalization::UnicodeNormalization;

/// YAML frontmatter for a skill.
///
/// This follows the agentskills specification strictly:
/// - frontmatter is required
/// - unknown keys are rejected (use `metadata` instead)
/// - `allowed-tools` is a space-delimited string (not a YAML list)
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct SkillFrontmatter {
    pub name: String,
    pub description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub license: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compatibility: Option<String>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "deserialize_metadata"
    )]
    pub metadata: Option<HashMap<String, String>>,
    /// Space-delimited list of allowed tools (spec).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allowed_tools: Option<String>,
}

/// Parsed SKILL.md document.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkillDoc {
    pub frontmatter: SkillFrontmatter,
    pub body: String,
}

#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum SkillParseError {
    #[error("missing YAML frontmatter (expected leading '---' fence)")]
    MissingFrontmatter,
    #[error("unterminated YAML frontmatter (missing closing '---' fence)")]
    UnterminatedFrontmatter,
    #[error("invalid YAML frontmatter: {0}")]
    InvalidYaml(String),
    #[error("invalid frontmatter: {0}")]
    InvalidFrontmatter(String),
}

/// Parse a SKILL.md (frontmatter + markdown body).
///
/// Frontmatter is required. When present, it must be YAML enclosed in `---` fences
/// at the top of the file.
pub fn parse_skill_md(input: &str) -> Result<SkillDoc, SkillParseError> {
    let input = input.replace("\r\n", "\n");
    if !input.starts_with("---\n") {
        return Err(SkillParseError::MissingFrontmatter);
    }

    // Find the terminating `---` line.
    // We only accept a fence at the beginning of a line.
    let mut lines = input.split('\n');
    let first = lines.next().unwrap_or_default();
    if first != "---" {
        return Err(SkillParseError::MissingFrontmatter);
    }

    let mut fm_lines = Vec::new();
    let mut found_end = false;
    for line in &mut lines {
        if line == "---" {
            found_end = true;
            break;
        }
        fm_lines.push(line);
    }

    if !found_end {
        return Err(SkillParseError::UnterminatedFrontmatter);
    }

    let fm_str = fm_lines.join("\n");
    let mut frontmatter = serde_yaml::from_str::<SkillFrontmatter>(&fm_str)
        .map_err(|e| SkillParseError::InvalidYaml(e.to_string()))?;
    // Spec: skill names are NFKC-normalized before validation and matching.
    frontmatter.name = normalize_skill_name(&frontmatter.name);
    validate_frontmatter(&frontmatter)?;
    let body = lines.collect::<Vec<_>>().join("\n");

    Ok(SkillDoc { frontmatter, body })
}

fn normalize_skill_name(name: &str) -> String {
    name.trim().nfkc().collect::<String>()
}

fn deserialize_metadata<'de, D>(
    deserializer: D,
) -> Result<Option<HashMap<String, String>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    // Spec: metadata is a mapping from string keys to string values.
    // We accept YAML scalars (string/number/bool/null) and stringify them,
    // but reject complex values (sequence/mapping).
    let raw: Option<HashMap<String, serde_yaml::Value>> =
        Option::<HashMap<String, serde_yaml::Value>>::deserialize(deserializer)?;
    let Some(raw) = raw else { return Ok(None) };

    let mut out = HashMap::with_capacity(raw.len());
    for (k, v) in raw {
        let s = match v {
            serde_yaml::Value::Null => "null".to_string(),
            serde_yaml::Value::Bool(b) => b.to_string(),
            serde_yaml::Value::Number(n) => n.to_string(),
            serde_yaml::Value::String(s) => s,
            serde_yaml::Value::Sequence(_) | serde_yaml::Value::Mapping(_) => {
                return Err(serde::de::Error::custom(format!(
                    "metadata value for key '{k}' must be a scalar"
                )));
            }
            serde_yaml::Value::Tagged(_) => {
                return Err(serde::de::Error::custom(format!(
                    "metadata value for key '{k}' must not be tagged"
                )));
            }
        };
        out.insert(k, s);
    }
    Ok(Some(out))
}

fn validate_frontmatter(fm: &SkillFrontmatter) -> Result<(), SkillParseError> {
    // name:
    // - 1..=64 chars
    // - unicode lowercase letters/digits and hyphens
    // - no leading/trailing '-'
    // - no consecutive '-'
    validate_skill_name(&fm.name)?;

    // description:
    // - 1..=1024 chars, non-empty after trimming
    let desc = fm.description.trim();
    if desc.is_empty() {
        return Err(SkillParseError::InvalidFrontmatter(
            "description must be non-empty".to_string(),
        ));
    }
    if desc.chars().count() > 1024 {
        return Err(SkillParseError::InvalidFrontmatter(
            "description must be <= 1024 characters".to_string(),
        ));
    }

    if let Some(c) = &fm.compatibility {
        let c = c.trim();
        if c.is_empty() {
            return Err(SkillParseError::InvalidFrontmatter(
                "compatibility must be non-empty if provided".to_string(),
            ));
        }
        if c.chars().count() > 500 {
            return Err(SkillParseError::InvalidFrontmatter(
                "compatibility must be <= 500 characters".to_string(),
            ));
        }
    }

    Ok(())
}

fn validate_skill_name(name: &str) -> Result<(), SkillParseError> {
    let n = normalize_skill_name(name);
    if n.is_empty() {
        return Err(SkillParseError::InvalidFrontmatter(
            "name must be non-empty".to_string(),
        ));
    }
    let len = n.chars().count();
    if len > 64 {
        return Err(SkillParseError::InvalidFrontmatter(
            "name must be <= 64 characters".to_string(),
        ));
    }
    if n != n.to_lowercase() {
        return Err(SkillParseError::InvalidFrontmatter(
            "name must be lowercase".to_string(),
        ));
    }
    if n.starts_with('-') {
        return Err(SkillParseError::InvalidFrontmatter(
            "name must not start with '-'".to_string(),
        ));
    }
    if n.ends_with('-') {
        return Err(SkillParseError::InvalidFrontmatter(
            "name must not end with '-'".to_string(),
        ));
    }
    if n.contains("--") {
        return Err(SkillParseError::InvalidFrontmatter(
            "name must not contain consecutive '-'".to_string(),
        ));
    }
    if !n.chars().all(|c| c.is_alphanumeric() || c == '-') {
        return Err(SkillParseError::InvalidFrontmatter(
            "name contains invalid characters (only letters, digits, and '-' are allowed)"
                .to_string(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_skill_md_requires_frontmatter() {
        let err = parse_skill_md("# Title\nBody").unwrap_err().to_string();
        assert!(err.contains("missing YAML frontmatter"));
    }

    #[test]
    fn parse_skill_md_with_frontmatter() {
        let input = r#"---
name: docx-processing
description: Test
allowed-tools: read_file other_tool(arg=1)
---
# Title
Body
"#;
        let doc = parse_skill_md(input).unwrap();
        assert_eq!(doc.frontmatter.name, "docx-processing");
        assert_eq!(doc.frontmatter.description, "Test");
        assert_eq!(
            doc.frontmatter.allowed_tools.as_deref(),
            Some("read_file other_tool(arg=1)")
        );
        assert!(doc.body.starts_with("# Title"));
    }

    #[test]
    fn parse_skill_md_invalid_yaml_is_error() {
        let input = r#"---
name: [unterminated
---
Body
"#;
        let err = parse_skill_md(input).unwrap_err().to_string();
        assert!(err.contains("invalid YAML"));
    }

    #[test]
    fn parse_skill_md_invalid_name_is_error() {
        let input = r#"---
name: Docx Processing
description: Test
---
Body
"#;
        let err = parse_skill_md(input).unwrap_err().to_string();
        assert!(err.contains("lowercase") || err.contains("invalid characters"));
    }

    #[test]
    fn parse_skill_md_allows_i18n_names_like_chinese() {
        let input = r#"---
name: 技能
description: ok
---
Body
"#;
        let doc = parse_skill_md(input).unwrap();
        assert_eq!(doc.frontmatter.name, "技能");
    }

    #[test]
    fn parse_skill_md_rejects_i18n_uppercase_names() {
        let input = r#"---
name: НАВЫК
description: ok
---
Body
"#;
        let err = parse_skill_md(input).unwrap_err().to_string();
        assert!(err.contains("lowercase"));
    }

    #[test]
    fn parse_skill_md_rejects_unknown_frontmatter_keys() {
        let input = r#"---
name: good-skill
description: ok
extra: no
---
Body
"#;
        let err = parse_skill_md(input).unwrap_err().to_string();
        assert!(err.contains("unknown field") || err.contains("Invalid"));
    }

    #[test]
    fn parse_skill_md_allows_non_string_metadata_values() {
        let input = r#"---
name: good-skill
description: ok
metadata:
  k: 1
---
Body
"#;
        let doc = parse_skill_md(input).unwrap();
        let meta = doc.frontmatter.metadata.expect("metadata present");
        assert_eq!(meta.get("k").map(String::as_str), Some("1"));
    }

    #[test]
    fn parse_skill_md_rejects_non_mapping_metadata() {
        let input = r#"---
name: good-skill
description: ok
metadata: 1
---
Body
"#;
        let err = parse_skill_md(input).unwrap_err().to_string();
        assert!(err.contains("invalid type") || err.contains("Invalid"));
    }

    #[test]
    fn parse_skill_md_rejects_blank_description() {
        let input = r#"---
name: good-skill
description: "   "
---
Body
"#;
        let err = parse_skill_md(input).unwrap_err().to_string();
        assert!(err.contains("description must be non-empty"));
    }

    #[test]
    fn parse_skill_md_rejects_overlong_description() {
        let long = "a".repeat(1025);
        let input = format!(
            "---\nname: good-skill\ndescription: \"{}\"\n---\nBody\n",
            long
        );
        let err = parse_skill_md(&input).unwrap_err().to_string();
        assert!(err.contains("description must be <= 1024"));
    }

    #[test]
    fn parse_skill_md_rejects_empty_compatibility() {
        let input = r#"---
name: good-skill
description: ok
compatibility: "  "
---
Body
"#;
        let err = parse_skill_md(input).unwrap_err().to_string();
        assert!(err.contains("compatibility must be non-empty"));
    }

    #[test]
    fn parse_skill_md_allows_crlf_newlines() {
        let input = "---\r\nname: good-skill\r\ndescription: ok\r\n---\r\nBody\r\n";
        let doc = parse_skill_md(input).unwrap();
        assert_eq!(doc.frontmatter.name, "good-skill");
        assert_eq!(doc.frontmatter.description, "ok");
        assert_eq!(doc.body.trim(), "Body");
    }
}
