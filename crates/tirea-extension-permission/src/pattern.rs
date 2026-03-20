//! Function-call-style permission rule patterns.
//!
//! A [`ToolCallPattern`] matches tool calls by tool name (glob/regex/exact) and
//! optionally by argument-level conditions on JSON fields.
//!
//! Syntax overview:
//! ```text
//! Bash                            exact tool, any args
//! Bash(*)                         explicit any args
//! Bash(npm *)                     primary arg glob
//! Edit(file_path ~ "src/**")      named field glob
//! Bash(command =~ "(?i)rm")       named field regex
//! mcp__github__*                  glob tool name
//! /mcp__(gh|gl)__.*/              regex tool name
//! Tool(a.b[*].c ~ "pat")         nested field path
//! Tool(f1 ~ "a", f2 = "b")       multi-field AND
//! ```

use std::fmt;

use serde::de::{self, Deserialize, Deserializer, Visitor};
use serde::ser::{Serialize, Serializer};

// ---------------------------------------------------------------------------
// Path segments for nested field access
// ---------------------------------------------------------------------------

/// A single segment in a dotted field path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PathSegment {
    /// Named object key: `connection`, `host`.
    Field(String),
    /// Specific array index: `[0]`, `[3]`.
    Index(usize),
    /// Any array element: `[*]`.
    AnyIndex,
    /// Any object key: `*` as a path segment (wildcard).
    Wildcard,
}

impl fmt::Display for PathSegment {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Field(name) => write!(f, "{name}"),
            Self::Index(i) => write!(f, "[{i}]"),
            Self::AnyIndex => write!(f, "[*]"),
            Self::Wildcard => write!(f, "*"),
        }
    }
}

// ---------------------------------------------------------------------------
// Match operators
// ---------------------------------------------------------------------------

/// Comparison operator for field conditions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatchOp {
    /// `~` — glob pattern match.
    Glob,
    /// `=` — exact string equality.
    Exact,
    /// `=~` — regex match.
    Regex,
    /// `!~` — negated glob.
    NotGlob,
    /// `!=` — not equal.
    NotExact,
    /// `!=~` — negated regex.
    NotRegex,
}

impl fmt::Display for MatchOp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Glob => write!(f, "~"),
            Self::Exact => write!(f, "="),
            Self::Regex => write!(f, "=~"),
            Self::NotGlob => write!(f, "!~"),
            Self::NotExact => write!(f, "!="),
            Self::NotRegex => write!(f, "!=~"),
        }
    }
}

// ---------------------------------------------------------------------------
// Field condition
// ---------------------------------------------------------------------------

/// A single field-level predicate: `path op "value"`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldCondition {
    /// Dotted path to the JSON field, e.g. `["queries", [*], "sql"]`.
    pub path: Vec<PathSegment>,
    /// Comparison operator.
    pub op: MatchOp,
    /// Pattern or literal value to compare against.
    pub value: String,
}

impl fmt::Display for FieldCondition {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Format path: field.sub[*].deep
        for (i, seg) in self.path.iter().enumerate() {
            match seg {
                PathSegment::Index(_) | PathSegment::AnyIndex => {
                    write!(f, "{seg}")?;
                }
                _ => {
                    if i > 0 {
                        write!(f, ".")?;
                    }
                    write!(f, "{seg}")?;
                }
            }
        }
        write!(f, " {} \"{}\"", self.op, self.value)
    }
}

// ---------------------------------------------------------------------------
// Tool matcher
// ---------------------------------------------------------------------------

/// How to match the tool name portion of a call.
#[derive(Debug, Clone)]
pub enum ToolMatcher {
    /// Exact string equality.
    Exact(String),
    /// Glob pattern (supports `*`, `?`, `[…]`).
    Glob(String),
    /// Compiled regex (source stored in the `Regex`).
    Regex(regex::Regex),
}

impl PartialEq for ToolMatcher {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Exact(a), Self::Exact(b)) => a == b,
            (Self::Glob(a), Self::Glob(b)) => a == b,
            (Self::Regex(a), Self::Regex(b)) => a.as_str() == b.as_str(),
            _ => false,
        }
    }
}

impl Eq for ToolMatcher {}

impl fmt::Display for ToolMatcher {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Exact(s) | Self::Glob(s) => write!(f, "{s}"),
            Self::Regex(re) => write!(f, "/{}/", re.as_str()),
        }
    }
}

// ---------------------------------------------------------------------------
// Argument matcher
// ---------------------------------------------------------------------------

/// How to match the arguments portion of a tool call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ArgMatcher {
    /// `(*)` or omitted — matches any arguments.
    Any,
    /// Positional shorthand: `(npm *)` → implicit glob on primary field.
    Primary { op: MatchOp, value: String },
    /// One or more named field conditions (AND semantics).
    Fields(Vec<FieldCondition>),
}

impl fmt::Display for ArgMatcher {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Any => write!(f, "*"),
            Self::Primary { op, value } => match op {
                // Short form for primary glob: just the value, no operator
                MatchOp::Glob => write!(f, "{value}"),
                _ => write!(f, "{op} \"{value}\""),
            },
            Self::Fields(conditions) => {
                for (i, cond) in conditions.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{cond}")?;
                }
                Ok(())
            }
        }
    }
}

// ---------------------------------------------------------------------------
// ToolCallPattern — the top-level pattern
// ---------------------------------------------------------------------------

/// A complete pattern matching tool calls by name and optionally by arguments.
///
/// Canonical string format: `ToolGlob(arg_conditions)`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolCallPattern {
    /// Tool name matcher (exact, glob, or regex).
    pub tool: ToolMatcher,
    /// Argument matcher (any, primary, or named fields).
    pub args: ArgMatcher,
}

impl fmt::Display for ToolCallPattern {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.tool)?;
        match &self.args {
            ArgMatcher::Any => Ok(()),
            other => write!(f, "({other})"),
        }
    }
}

// ---------------------------------------------------------------------------
// Serde — round-trips through the canonical pattern string
// ---------------------------------------------------------------------------

impl Serialize for ToolCallPattern {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for ToolCallPattern {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct PatternVisitor;

        impl<'de> Visitor<'de> for PatternVisitor {
            type Value = ToolCallPattern;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a tool call pattern string like \"Bash(npm *)\"")
            }

            fn visit_str<E: de::Error>(self, v: &str) -> Result<Self::Value, E> {
                crate::parser::parse_pattern(v).map_err(de::Error::custom)
            }
        }

        deserializer.deserialize_str(PatternVisitor)
    }
}

// ---------------------------------------------------------------------------
// Constructors
// ---------------------------------------------------------------------------

impl ToolCallPattern {
    /// Exact tool name, any args.
    #[must_use]
    pub fn tool(name: impl Into<String>) -> Self {
        Self {
            tool: ToolMatcher::Exact(name.into()),
            args: ArgMatcher::Any,
        }
    }

    /// Exact tool name with a primary glob pattern.
    #[must_use]
    pub fn tool_with_primary(name: impl Into<String>, pattern: impl Into<String>) -> Self {
        Self {
            tool: ToolMatcher::Exact(name.into()),
            args: ArgMatcher::Primary {
                op: MatchOp::Glob,
                value: pattern.into(),
            },
        }
    }

    /// Glob tool name, any args.
    #[must_use]
    pub fn tool_glob(pattern: impl Into<String>) -> Self {
        Self {
            tool: ToolMatcher::Glob(pattern.into()),
            args: ArgMatcher::Any,
        }
    }

    /// Set argument matcher.
    #[must_use]
    pub fn with_args(mut self, args: ArgMatcher) -> Self {
        self.args = args;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_exact_tool_any_args() {
        let p = ToolCallPattern::tool("Bash");
        assert_eq!(p.to_string(), "Bash");
    }

    #[test]
    fn display_glob_tool() {
        let p = ToolCallPattern::tool_glob("mcp__github__*");
        assert_eq!(p.to_string(), "mcp__github__*");
    }

    #[test]
    fn display_regex_tool() {
        let p = ToolCallPattern {
            tool: ToolMatcher::Regex(regex::Regex::new(r"mcp__(gh|gl)__.*").unwrap()),
            args: ArgMatcher::Any,
        };
        assert_eq!(p.to_string(), "/mcp__(gh|gl)__.*/");
    }

    #[test]
    fn display_primary_glob() {
        let p = ToolCallPattern::tool_with_primary("Bash", "npm *");
        assert_eq!(p.to_string(), "Bash(npm *)");
    }

    #[test]
    fn display_primary_exact() {
        let p = ToolCallPattern {
            tool: ToolMatcher::Exact("Bash".into()),
            args: ArgMatcher::Primary {
                op: MatchOp::Exact,
                value: "git status".into(),
            },
        };
        assert_eq!(p.to_string(), r#"Bash(= "git status")"#);
    }

    #[test]
    fn display_named_field_glob() {
        let p = ToolCallPattern {
            tool: ToolMatcher::Exact("Edit".into()),
            args: ArgMatcher::Fields(vec![FieldCondition {
                path: vec![PathSegment::Field("file_path".into())],
                op: MatchOp::Glob,
                value: "src/**/*.rs".into(),
            }]),
        };
        assert_eq!(p.to_string(), r#"Edit(file_path ~ "src/**/*.rs")"#);
    }

    #[test]
    fn display_nested_path() {
        let p = ToolCallPattern {
            tool: ToolMatcher::Exact("mcp__db__query".into()),
            args: ArgMatcher::Fields(vec![FieldCondition {
                path: vec![
                    PathSegment::Field("queries".into()),
                    PathSegment::AnyIndex,
                    PathSegment::Field("sql".into()),
                ],
                op: MatchOp::Regex,
                value: "(?i)DROP".into(),
            }]),
        };
        assert_eq!(
            p.to_string(),
            r#"mcp__db__query(queries[*].sql =~ "(?i)DROP")"#
        );
    }

    #[test]
    fn display_multi_field() {
        let p = ToolCallPattern {
            tool: ToolMatcher::Exact("Bash".into()),
            args: ArgMatcher::Fields(vec![
                FieldCondition {
                    path: vec![PathSegment::Field("command".into())],
                    op: MatchOp::Glob,
                    value: "curl *".into(),
                },
                FieldCondition {
                    path: vec![PathSegment::Field("command".into())],
                    op: MatchOp::Glob,
                    value: "*| *".into(),
                },
            ]),
        };
        assert_eq!(
            p.to_string(),
            r#"Bash(command ~ "curl *", command ~ "*| *")"#
        );
    }

    #[test]
    fn display_wildcard_path_segment() {
        let p = ToolCallPattern {
            tool: ToolMatcher::Glob("mcp__*".into()),
            args: ArgMatcher::Fields(vec![FieldCondition {
                path: vec![PathSegment::Wildcard, PathSegment::Field("password".into())],
                op: MatchOp::Regex,
                value: ".*".into(),
            }]),
        };
        assert_eq!(p.to_string(), r#"mcp__*(*.password =~ ".*")"#);
    }

    #[test]
    fn equality_for_regex_tool_matcher() {
        let a = ToolMatcher::Regex(regex::Regex::new("abc").unwrap());
        let b = ToolMatcher::Regex(regex::Regex::new("abc").unwrap());
        let c = ToolMatcher::Regex(regex::Regex::new("xyz").unwrap());
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn serde_round_trip() {
        let p = ToolCallPattern::tool_with_primary("Bash", "npm *");
        let json = serde_json::to_string(&p).unwrap();
        assert_eq!(json, r#""Bash(npm *)""#);
        // Deserialization is tested after parser.rs is implemented
    }
}
