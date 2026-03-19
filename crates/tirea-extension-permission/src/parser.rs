//! Parse permission rule pattern strings into [`ToolCallPattern`].
//!
//! Grammar (informal):
//!
//! ```text
//! pattern      = tool_part [ "(" arg_part ")" ]
//! tool_part    = "/" regex_body "/"
//!              | glob_or_exact
//! arg_part     = "*"
//!              | field_cond ("," field_cond)*
//!              | primary_value          (no operator → primary glob)
//! field_cond   = field_path op value
//! field_path   = segment ("." segment)*
//! segment      = ident [ "[" index "]" ]
//! index        = number | "*"
//! op           = "=~" | "!=~" | "!~" | "!=" | "~" | "="
//! value        = '"' ... '"'
//! ```

use crate::pattern::{
    ArgMatcher, FieldCondition, MatchOp, PathSegment, ToolCallPattern, ToolMatcher,
};
use std::fmt;

/// Error returned when a pattern string cannot be parsed.
#[derive(Debug, Clone)]
pub struct PatternParseError {
    pub message: String,
    pub position: usize,
}

impl fmt::Display for PatternParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "parse error at {}: {}", self.position, self.message)
    }
}

impl std::error::Error for PatternParseError {}

// ---------------------------------------------------------------------------
// Cursor
// ---------------------------------------------------------------------------

struct Cursor<'a> {
    input: &'a str,
    pos: usize,
}

impl<'a> Cursor<'a> {
    fn new(input: &'a str) -> Self {
        Self { input, pos: 0 }
    }

    fn remaining(&self) -> &'a str {
        &self.input[self.pos..]
    }

    fn is_empty(&self) -> bool {
        self.pos >= self.input.len()
    }

    fn peek(&self) -> Option<char> {
        self.remaining().chars().next()
    }

    fn advance(&mut self, n: usize) {
        self.pos += n;
    }

    fn skip_whitespace(&mut self) {
        while let Some(c) = self.peek() {
            if c.is_ascii_whitespace() {
                self.advance(c.len_utf8());
            } else {
                break;
            }
        }
    }

    fn expect(&mut self, ch: char) -> Result<(), PatternParseError> {
        self.skip_whitespace();
        match self.peek() {
            Some(c) if c == ch => {
                self.advance(c.len_utf8());
                Ok(())
            }
            other => Err(self.error(format!(
                "expected '{}', found {}",
                ch,
                match other {
                    Some(c) => format!("'{c}'"),
                    None => "end of input".to_string(),
                }
            ))),
        }
    }

    fn error(&self, message: impl Into<String>) -> PatternParseError {
        PatternParseError {
            message: message.into(),
            position: self.pos,
        }
    }
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Parse a pattern string into a [`ToolCallPattern`].
///
/// # Examples
///
/// ```ignore
/// parse_pattern("Bash")                              // exact tool, any args
/// parse_pattern("Bash(npm *)")                       // primary glob
/// parse_pattern("Edit(file_path ~ \"src/**\")")      // named field glob
/// parse_pattern("mcp__github__*")                    // glob tool name
/// parse_pattern("/mcp__(gh|gl)__.*/")                // regex tool name
/// parse_pattern("Bash(command ~ \"a\", command ~ \"b\")")  // multi-field AND
/// ```
pub fn parse_pattern(input: &str) -> Result<ToolCallPattern, PatternParseError> {
    let mut cursor = Cursor::new(input.trim());

    let tool = parse_tool_part(&mut cursor)?;
    cursor.skip_whitespace();

    let args = if cursor.peek() == Some('(') {
        cursor.advance(1);
        let args = parse_arg_part(&mut cursor)?;
        cursor.expect(')')?;
        args
    } else {
        ArgMatcher::Any
    };

    cursor.skip_whitespace();
    if !cursor.is_empty() {
        return Err(cursor.error(format!("unexpected trailing: '{}'", cursor.remaining())));
    }

    Ok(ToolCallPattern { tool, args })
}

// ---------------------------------------------------------------------------
// Tool part
// ---------------------------------------------------------------------------

fn parse_tool_part(cursor: &mut Cursor<'_>) -> Result<ToolMatcher, PatternParseError> {
    cursor.skip_whitespace();
    if cursor.peek() == Some('/') {
        // Regex: /pattern/
        cursor.advance(1);
        let start = cursor.pos;
        let mut depth = 0u32;
        while let Some(c) = cursor.peek() {
            match c {
                '\\' => {
                    cursor.advance(1);
                    // skip escaped char
                    if cursor.peek().is_some() {
                        cursor.advance(1);
                    }
                }
                '(' => {
                    depth += 1;
                    cursor.advance(1);
                }
                ')' => {
                    depth = depth.saturating_sub(1);
                    cursor.advance(1);
                }
                '/' if depth == 0 => break,
                _ => cursor.advance(c.len_utf8()),
            }
        }
        let body = &cursor.input[start..cursor.pos];
        if body.is_empty() {
            return Err(cursor.error("empty regex pattern"));
        }
        cursor.expect('/')?;
        let re =
            regex::Regex::new(body).map_err(|e| cursor.error(format!("invalid regex: {e}")))?;
        Ok(ToolMatcher::Regex(re))
    } else {
        // Glob or exact: read until '(' or end
        let start = cursor.pos;
        while let Some(c) = cursor.peek() {
            if c == '(' || c.is_ascii_whitespace() {
                break;
            }
            cursor.advance(c.len_utf8());
        }
        let name = &cursor.input[start..cursor.pos];
        if name.is_empty() {
            return Err(cursor.error("empty tool name"));
        }
        if has_glob_chars(name) {
            Ok(ToolMatcher::Glob(name.to_string()))
        } else {
            Ok(ToolMatcher::Exact(name.to_string()))
        }
    }
}

fn has_glob_chars(s: &str) -> bool {
    s.contains('*') || s.contains('?') || s.contains('[')
}

// ---------------------------------------------------------------------------
// Arg part
// ---------------------------------------------------------------------------

fn parse_arg_part(cursor: &mut Cursor<'_>) -> Result<ArgMatcher, PatternParseError> {
    cursor.skip_whitespace();

    // Check for `*` (any)
    if cursor.peek() == Some('*') {
        let after = cursor.remaining().get(1..2);
        // `*` followed by `)` or end means "any"
        if after.is_none_or(|s| {
            let c = s.chars().next().unwrap_or(')');
            c == ')' || c.is_ascii_whitespace()
        }) {
            cursor.advance(1);
            cursor.skip_whitespace();
            return Ok(ArgMatcher::Any);
        }
    }

    // Try to detect if this is a named field condition or a primary value.
    // Named fields have: `identifier[.identifier]* op value`
    // Primary values have no operator prefix.
    if looks_like_field_conditions(cursor.remaining()) {
        parse_field_conditions(cursor)
    } else {
        parse_primary_value(cursor)
    }
}

/// Heuristic: does the content look like `fieldName op "value"` rather than
/// a bare primary value like `npm *`?
///
/// Look for a field identifier followed by a match operator.
fn looks_like_field_conditions(s: &str) -> bool {
    let s = s.trim();
    // Walk past identifier chars and path separators to find an operator.
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i] as char;
        if c.is_ascii_alphanumeric() || c == '_' || c == '.' {
            i += 1;
        } else if c == '[' {
            // Skip bracket contents: [0], [*], etc.
            i += 1;
            while i < bytes.len() && bytes[i] != b']' {
                i += 1;
            }
            if i < bytes.len() {
                i += 1; // skip ']'
            }
        } else if c == '*' {
            // Wildcard path segment — check if followed by `.` or `[` (path continuation)
            i += 1;
            if i < bytes.len() && (bytes[i] == b'.' || bytes[i] == b'[') {
                continue;
            }
            // Standalone `*` — stop scanning
            break;
        } else {
            break;
        }
    }
    // Skip whitespace
    while i < bytes.len() && (bytes[i] as char).is_ascii_whitespace() {
        i += 1;
    }
    // Check if next chars form an operator
    let remaining = &s[i..];
    remaining.starts_with("~")
        || remaining.starts_with("=")
        || remaining.starts_with("!~")
        || remaining.starts_with("!=")
}

fn parse_field_conditions(cursor: &mut Cursor<'_>) -> Result<ArgMatcher, PatternParseError> {
    let mut conditions = Vec::new();
    loop {
        cursor.skip_whitespace();
        conditions.push(parse_single_field_condition(cursor)?);
        cursor.skip_whitespace();
        if cursor.peek() == Some(',') {
            cursor.advance(1);
        } else {
            break;
        }
    }
    Ok(ArgMatcher::Fields(conditions))
}

fn parse_single_field_condition(
    cursor: &mut Cursor<'_>,
) -> Result<FieldCondition, PatternParseError> {
    cursor.skip_whitespace();
    let path = parse_field_path(cursor)?;
    cursor.skip_whitespace();
    let op = parse_match_op(cursor)?;
    cursor.skip_whitespace();
    let value = parse_quoted_value(cursor)?;
    Ok(FieldCondition { path, op, value })
}

// ---------------------------------------------------------------------------
// Field path: `field.sub[*].deep` or `*` or `*.password`
// ---------------------------------------------------------------------------

fn parse_field_path(cursor: &mut Cursor<'_>) -> Result<Vec<PathSegment>, PatternParseError> {
    let mut segments = Vec::new();
    loop {
        cursor.skip_whitespace();
        if cursor.peek() == Some('*') {
            // Wildcard segment — check if followed by `.` or `[` (path continuation)
            // or if it's the first/middle segment
            cursor.advance(1);
            segments.push(PathSegment::Wildcard);
        } else {
            let ident = parse_identifier(cursor)?;
            segments.push(PathSegment::Field(ident));
        }

        // Check for array index: [0] or [*]
        while cursor.peek() == Some('[') {
            cursor.advance(1); // consume '['
            cursor.skip_whitespace();
            if cursor.peek() == Some('*') {
                cursor.advance(1);
                segments.push(PathSegment::AnyIndex);
            } else {
                let idx = parse_usize(cursor)?;
                segments.push(PathSegment::Index(idx));
            }
            cursor.expect(']')?;
        }

        // Check for dot continuation
        if cursor.peek() == Some('.') {
            cursor.advance(1);
        } else {
            break;
        }
    }
    Ok(segments)
}

fn parse_identifier(cursor: &mut Cursor<'_>) -> Result<String, PatternParseError> {
    let start = cursor.pos;
    while let Some(c) = cursor.peek() {
        if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
            cursor.advance(1);
        } else {
            break;
        }
    }
    let ident = &cursor.input[start..cursor.pos];
    if ident.is_empty() {
        return Err(cursor.error("expected identifier"));
    }
    Ok(ident.to_string())
}

fn parse_usize(cursor: &mut Cursor<'_>) -> Result<usize, PatternParseError> {
    let start = cursor.pos;
    while let Some(c) = cursor.peek() {
        if c.is_ascii_digit() {
            cursor.advance(1);
        } else {
            break;
        }
    }
    let digits = &cursor.input[start..cursor.pos];
    digits
        .parse::<usize>()
        .map_err(|_| cursor.error(format!("invalid index: '{digits}'")))
}

// ---------------------------------------------------------------------------
// Match operator
// ---------------------------------------------------------------------------

fn parse_match_op(cursor: &mut Cursor<'_>) -> Result<MatchOp, PatternParseError> {
    let remaining = cursor.remaining();
    // Order matters: try longer operators first
    if remaining.starts_with("!=~") {
        cursor.advance(3);
        Ok(MatchOp::NotRegex)
    } else if remaining.starts_with("!=") {
        cursor.advance(2);
        Ok(MatchOp::NotExact)
    } else if remaining.starts_with("!~") {
        cursor.advance(2);
        Ok(MatchOp::NotGlob)
    } else if remaining.starts_with("=~") {
        cursor.advance(2);
        Ok(MatchOp::Regex)
    } else if remaining.starts_with('~') {
        cursor.advance(1);
        Ok(MatchOp::Glob)
    } else if remaining.starts_with('=') {
        cursor.advance(1);
        Ok(MatchOp::Exact)
    } else {
        Err(cursor.error("expected operator: ~, =, =~, !~, !=, or !=~"))
    }
}

// ---------------------------------------------------------------------------
// Quoted value
// ---------------------------------------------------------------------------

fn parse_quoted_value(cursor: &mut Cursor<'_>) -> Result<String, PatternParseError> {
    cursor.skip_whitespace();
    if cursor.peek() != Some('"') {
        return Err(cursor.error("expected '\"' to start value"));
    }
    cursor.advance(1); // consume opening quote
    let mut value = String::new();
    loop {
        match cursor.peek() {
            None => return Err(cursor.error("unterminated string literal")),
            Some('"') => {
                cursor.advance(1);
                break;
            }
            Some('\\') => {
                cursor.advance(1);
                match cursor.peek() {
                    Some(c @ ('"' | '\\')) => {
                        value.push(c);
                        cursor.advance(1);
                    }
                    Some(c) => {
                        // Keep backslash for regex patterns like \b, \s etc.
                        value.push('\\');
                        value.push(c);
                        cursor.advance(c.len_utf8());
                    }
                    None => return Err(cursor.error("unterminated escape sequence")),
                }
            }
            Some(c) => {
                value.push(c);
                cursor.advance(c.len_utf8());
            }
        }
    }
    Ok(value)
}

// ---------------------------------------------------------------------------
// Primary value: bare glob pattern without quotes, like `npm *`
// ---------------------------------------------------------------------------

fn parse_primary_value(cursor: &mut Cursor<'_>) -> Result<ArgMatcher, PatternParseError> {
    cursor.skip_whitespace();
    let start = cursor.pos;

    // Read until ')' — the content is the primary glob pattern
    let mut depth = 0u32;
    while let Some(c) = cursor.peek() {
        match c {
            '(' => {
                depth += 1;
                cursor.advance(1);
            }
            ')' if depth > 0 => {
                depth -= 1;
                cursor.advance(1);
            }
            ')' => break,
            _ => cursor.advance(c.len_utf8()),
        }
    }

    let value = cursor.input[start..cursor.pos].trim();
    if value.is_empty() {
        return Err(cursor.error("empty primary pattern"));
    }
    Ok(ArgMatcher::Primary {
        op: MatchOp::Glob,
        value: value.to_string(),
    })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pattern::*;

    #[test]
    fn parse_exact_tool_only() {
        let p = parse_pattern("Bash").unwrap();
        assert_eq!(p.tool, ToolMatcher::Exact("Bash".into()));
        assert_eq!(p.args, ArgMatcher::Any);
    }

    #[test]
    fn parse_glob_tool_only() {
        let p = parse_pattern("mcp__github__*").unwrap();
        assert_eq!(p.tool, ToolMatcher::Glob("mcp__github__*".into()));
        assert_eq!(p.args, ArgMatcher::Any);
    }

    #[test]
    fn parse_regex_tool() {
        let p = parse_pattern(r"/mcp__(gh|gl)__.*/").unwrap();
        assert!(matches!(p.tool, ToolMatcher::Regex(_)));
        if let ToolMatcher::Regex(re) = &p.tool {
            assert_eq!(re.as_str(), r"mcp__(gh|gl)__.*");
        }
        assert_eq!(p.args, ArgMatcher::Any);
    }

    #[test]
    fn parse_any_args_explicit() {
        let p = parse_pattern("Bash(*)").unwrap();
        assert_eq!(p.tool, ToolMatcher::Exact("Bash".into()));
        assert_eq!(p.args, ArgMatcher::Any);
    }

    #[test]
    fn parse_primary_glob() {
        let p = parse_pattern("Bash(npm *)").unwrap();
        assert_eq!(p.tool, ToolMatcher::Exact("Bash".into()));
        assert_eq!(
            p.args,
            ArgMatcher::Primary {
                op: MatchOp::Glob,
                value: "npm *".into()
            }
        );
    }

    #[test]
    fn parse_primary_glob_git_status() {
        let p = parse_pattern("Bash(git status)").unwrap();
        assert_eq!(
            p.args,
            ArgMatcher::Primary {
                op: MatchOp::Glob,
                value: "git status".into()
            }
        );
    }

    #[test]
    fn parse_named_field_glob() {
        let p = parse_pattern(r#"Edit(file_path ~ "src/**/*.rs")"#).unwrap();
        assert_eq!(p.tool, ToolMatcher::Exact("Edit".into()));
        if let ArgMatcher::Fields(conditions) = &p.args {
            assert_eq!(conditions.len(), 1);
            assert_eq!(
                conditions[0].path,
                vec![PathSegment::Field("file_path".into())]
            );
            assert_eq!(conditions[0].op, MatchOp::Glob);
            assert_eq!(conditions[0].value, "src/**/*.rs");
        } else {
            panic!("expected Fields");
        }
    }

    #[test]
    fn parse_named_field_regex() {
        let p = parse_pattern(r#"Bash(command =~ "(?i)eval|exec")"#).unwrap();
        if let ArgMatcher::Fields(conditions) = &p.args {
            assert_eq!(conditions[0].op, MatchOp::Regex);
            assert_eq!(conditions[0].value, "(?i)eval|exec");
        } else {
            panic!("expected Fields");
        }
    }

    #[test]
    fn parse_named_field_not_glob() {
        let p = parse_pattern(r#"Bash(command !~ "npm *")"#).unwrap();
        if let ArgMatcher::Fields(conditions) = &p.args {
            assert_eq!(conditions[0].op, MatchOp::NotGlob);
        } else {
            panic!("expected Fields");
        }
    }

    #[test]
    fn parse_named_field_exact() {
        let p = parse_pattern(r#"Bash(command = "ls")"#).unwrap();
        if let ArgMatcher::Fields(conditions) = &p.args {
            assert_eq!(conditions[0].op, MatchOp::Exact);
            assert_eq!(conditions[0].value, "ls");
        } else {
            panic!("expected Fields");
        }
    }

    #[test]
    fn parse_named_field_not_exact() {
        let p = parse_pattern(r#"Bash(command != "rm")"#).unwrap();
        if let ArgMatcher::Fields(conditions) = &p.args {
            assert_eq!(conditions[0].op, MatchOp::NotExact);
        } else {
            panic!("expected Fields");
        }
    }

    #[test]
    fn parse_named_field_not_regex() {
        let p = parse_pattern(r#"Bash(command !=~ "danger")"#).unwrap();
        if let ArgMatcher::Fields(conditions) = &p.args {
            assert_eq!(conditions[0].op, MatchOp::NotRegex);
        } else {
            panic!("expected Fields");
        }
    }

    #[test]
    fn parse_nested_path_any_index() {
        let p = parse_pattern(r#"mcp__db__query(queries[*].sql =~ "(?i)DROP")"#).unwrap();
        if let ArgMatcher::Fields(conditions) = &p.args {
            assert_eq!(
                conditions[0].path,
                vec![
                    PathSegment::Field("queries".into()),
                    PathSegment::AnyIndex,
                    PathSegment::Field("sql".into()),
                ]
            );
        } else {
            panic!("expected Fields");
        }
    }

    #[test]
    fn parse_nested_path_specific_index() {
        let p = parse_pattern(r#"Tool(items[0].name = "test")"#).unwrap();
        if let ArgMatcher::Fields(conditions) = &p.args {
            assert_eq!(
                conditions[0].path,
                vec![
                    PathSegment::Field("items".into()),
                    PathSegment::Index(0),
                    PathSegment::Field("name".into()),
                ]
            );
        } else {
            panic!("expected Fields");
        }
    }

    #[test]
    fn parse_wildcard_path_segment() {
        let p = parse_pattern(r#"mcp__*(*.password =~ ".*")"#).unwrap();
        assert_eq!(p.tool, ToolMatcher::Glob("mcp__*".into()));
        if let ArgMatcher::Fields(conditions) = &p.args {
            assert_eq!(
                conditions[0].path,
                vec![PathSegment::Wildcard, PathSegment::Field("password".into()),]
            );
        } else {
            panic!("expected Fields");
        }
    }

    #[test]
    fn parse_multi_field_and() {
        let p = parse_pattern(r#"Bash(command ~ "curl *", command ~ "*| *")"#).unwrap();
        if let ArgMatcher::Fields(conditions) = &p.args {
            assert_eq!(conditions.len(), 2);
            assert_eq!(conditions[0].value, "curl *");
            assert_eq!(conditions[1].value, "*| *");
        } else {
            panic!("expected Fields");
        }
    }

    #[test]
    fn parse_complex_multi_field() {
        let p = parse_pattern(
            r#"mcp__db__query(connection.host = "localhost", queries[*].sql =~ "^SELECT\b")"#,
        )
        .unwrap();
        if let ArgMatcher::Fields(conditions) = &p.args {
            assert_eq!(conditions.len(), 2);
            assert_eq!(
                conditions[0].path,
                vec![
                    PathSegment::Field("connection".into()),
                    PathSegment::Field("host".into()),
                ]
            );
            assert_eq!(conditions[0].op, MatchOp::Exact);
            assert_eq!(conditions[0].value, "localhost");

            assert_eq!(
                conditions[1].path,
                vec![
                    PathSegment::Field("queries".into()),
                    PathSegment::AnyIndex,
                    PathSegment::Field("sql".into()),
                ]
            );
            assert_eq!(conditions[1].op, MatchOp::Regex);
        } else {
            panic!("expected Fields");
        }
    }

    #[test]
    fn parse_display_round_trip_exact_tool() {
        let original = "Bash";
        let p = parse_pattern(original).unwrap();
        assert_eq!(p.to_string(), original);
    }

    #[test]
    fn parse_display_round_trip_glob_tool() {
        let original = "mcp__github__*";
        let p = parse_pattern(original).unwrap();
        assert_eq!(p.to_string(), original);
    }

    #[test]
    fn parse_display_round_trip_primary() {
        let original = "Bash(npm *)";
        let p = parse_pattern(original).unwrap();
        assert_eq!(p.to_string(), original);
    }

    #[test]
    fn parse_display_round_trip_named_field() {
        let original = r#"Edit(file_path ~ "src/**")"#;
        let p = parse_pattern(original).unwrap();
        assert_eq!(p.to_string(), original);
    }

    #[test]
    fn parse_display_round_trip_nested() {
        let original = r#"mcp__db__query(queries[*].sql =~ "(?i)DROP")"#;
        let p = parse_pattern(original).unwrap();
        assert_eq!(p.to_string(), original);
    }

    #[test]
    fn error_empty_input() {
        assert!(parse_pattern("").is_err());
    }

    #[test]
    fn error_unmatched_paren() {
        assert!(parse_pattern("Bash(npm *").is_err());
    }

    #[test]
    fn error_empty_regex() {
        assert!(parse_pattern("//").is_err());
    }

    #[test]
    fn error_invalid_regex() {
        assert!(parse_pattern("/[invalid/").is_err());
    }

    #[test]
    fn error_trailing_content() {
        assert!(parse_pattern("Bash extra").is_err());
    }

    #[test]
    fn serde_deserialize_round_trip() {
        let json_str = r#""Bash(npm *)""#;
        let p: ToolCallPattern = serde_json::from_str(json_str).unwrap();
        assert_eq!(p.tool, ToolMatcher::Exact("Bash".into()));
        assert_eq!(
            p.args,
            ArgMatcher::Primary {
                op: MatchOp::Glob,
                value: "npm *".into()
            }
        );
        let re_serialized = serde_json::to_string(&p).unwrap();
        assert_eq!(re_serialized, json_str);
    }

    #[test]
    fn serde_deserialize_named_field() {
        let json_str = r#""Edit(file_path ~ \"src/**\")""#;
        let p: ToolCallPattern = serde_json::from_str(json_str).unwrap();
        assert_eq!(p.to_string(), r#"Edit(file_path ~ "src/**")"#);
    }
}
