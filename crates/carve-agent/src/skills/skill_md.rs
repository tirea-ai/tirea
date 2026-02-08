use serde::{Deserialize, Serialize};

/// YAML frontmatter for a skill.
///
/// This is intentionally minimal and tolerant (unknown keys are ignored).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct SkillFrontmatter {
    pub name: Option<String>,
    pub description: Option<String>,
    #[serde(default, alias = "allowed_tools", alias = "allowedTools")]
    pub allowed_tools: Vec<String>,
}

/// Parsed SKILL.md document.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkillDoc {
    pub frontmatter: SkillFrontmatter,
    pub body: String,
}

/// Parse a SKILL.md (frontmatter + markdown body).
///
/// Frontmatter is optional. When present, it must be YAML enclosed in `---` fences
/// at the top of the file.
pub fn parse_skill_md(input: &str) -> SkillDoc {
    let input = input.replace("\r\n", "\n");
    if !input.starts_with("---\n") {
        return SkillDoc {
            frontmatter: SkillFrontmatter::default(),
            body: input,
        };
    }

    // Find the terminating `---` line.
    // We only accept a fence at the beginning of a line.
    let mut lines = input.split('\n');
    let first = lines.next().unwrap_or_default();
    if first != "---" {
        return SkillDoc {
            frontmatter: SkillFrontmatter::default(),
            body: input,
        };
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
        return SkillDoc {
            frontmatter: SkillFrontmatter::default(),
            body: input,
        };
    }

    let fm_str = fm_lines.join("\n");
    let frontmatter = serde_yaml::from_str::<SkillFrontmatter>(&fm_str).unwrap_or_default();
    let body = lines.collect::<Vec<_>>().join("\n");

    SkillDoc { frontmatter, body }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_skill_md_without_frontmatter() {
        let doc = parse_skill_md("# Title\nBody");
        assert_eq!(doc.frontmatter.name, None);
        assert!(doc.body.contains("Title"));
    }

    #[test]
    fn parse_skill_md_with_frontmatter() {
        let input = r#"---
name: Docx
description: Test
allowed-tools:
  - read_file
---
# Title
Body
"#;
        let doc = parse_skill_md(input);
        assert_eq!(doc.frontmatter.name.as_deref(), Some("Docx"));
        assert_eq!(doc.frontmatter.description.as_deref(), Some("Test"));
        assert_eq!(doc.frontmatter.allowed_tools, vec!["read_file".to_string()]);
        assert!(doc.body.starts_with("# Title"));
    }

    #[test]
    fn parse_skill_md_invalid_yaml_falls_back_to_default_frontmatter() {
        let input = r#"---
name: [unterminated
---
Body
"#;
        let doc = parse_skill_md(input);
        assert_eq!(doc.frontmatter.name, None);
        assert_eq!(doc.body.trim(), "Body");
    }
}
