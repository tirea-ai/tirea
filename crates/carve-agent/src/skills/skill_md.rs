use serde::{Deserialize, Serialize};
use std::collections::HashMap;

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
    #[serde(default, skip_serializing_if = "Option::is_none")]
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
    let frontmatter = serde_yaml::from_str::<SkillFrontmatter>(&fm_str)
        .map_err(|e| SkillParseError::InvalidYaml(e.to_string()))?;
    validate_frontmatter(&frontmatter)?;
    let body = lines.collect::<Vec<_>>().join("\n");

    Ok(SkillDoc { frontmatter, body })
}

fn validate_frontmatter(fm: &SkillFrontmatter) -> Result<(), SkillParseError> {
    // name:
    // - 1..=64 chars
    // - lowercase kebab-case: [a-z0-9-]
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

    if let Some(s) = &fm.allowed_tools {
        // Spec: space-delimited list.
        // Reject strings that contain no tools (after splitting).
        if s.split_whitespace().next().is_none() {
            return Err(SkillParseError::InvalidFrontmatter(
                "allowed-tools must contain at least one tool token".to_string(),
            ));
        }
    }

    Ok(())
}

fn validate_skill_name(name: &str) -> Result<(), SkillParseError> {
    let n = name.trim();
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
    let mut prev_hyphen = false;
    for (i, ch) in n.chars().enumerate() {
        let is_hyphen = ch == '-';
        let is_ok = ch.is_ascii_lowercase() || ch.is_ascii_digit() || is_hyphen;
        if !is_ok {
            return Err(SkillParseError::InvalidFrontmatter(
                "name must be lowercase kebab-case ([a-z0-9-])".to_string(),
            ));
        }
        if i == 0 && is_hyphen {
            return Err(SkillParseError::InvalidFrontmatter(
                "name must not start with '-'".to_string(),
            ));
        }
        if prev_hyphen && is_hyphen {
            return Err(SkillParseError::InvalidFrontmatter(
                "name must not contain consecutive '-'".to_string(),
            ));
        }
        prev_hyphen = is_hyphen;
    }
    if n.ends_with('-') {
        return Err(SkillParseError::InvalidFrontmatter(
            "name must not end with '-'".to_string(),
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
        assert!(err.contains("kebab-case"));
    }
}
