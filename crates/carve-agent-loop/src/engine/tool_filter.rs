use carve_state::ScopeState;
use serde_json::Value;

pub fn is_tool_allowed(
    tool_id: &str,
    allowed: Option<&[String]>,
    excluded: Option<&[String]>,
) -> bool {
    if let Some(allowed) = allowed {
        if !allowed.iter().any(|t| t == tool_id) {
            return false;
        }
    }
    if let Some(excluded) = excluded {
        if excluded.iter().any(|t| t == tool_id) {
            return false;
        }
    }
    true
}

fn parse_scope_filter(values: Option<&Value>) -> Option<Vec<String>> {
    let arr = values?.as_array()?;
    let parsed: Vec<String> = arr
        .iter()
        .filter_map(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect();
    Some(parsed)
}

pub fn is_scope_allowed(
    scope: Option<&ScopeState>,
    id: &str,
    allowed_key: &str,
    excluded_key: &str,
) -> bool {
    let allowed = parse_scope_filter(scope.and_then(|s| s.value(allowed_key)));
    let excluded = parse_scope_filter(scope.and_then(|s| s.value(excluded_key)));
    is_tool_allowed(id, allowed.as_deref(), excluded.as_deref())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contracts::runtime::{SCOPE_ALLOWED_TOOLS_KEY, SCOPE_EXCLUDED_TOOLS_KEY};

    #[test]
    fn test_is_tool_allowed_allows_when_no_filters() {
        assert!(is_tool_allowed("a", None, None));
    }

    #[test]
    fn test_is_tool_allowed_allows_when_in_allowed() {
        let allowed = vec!["a".to_string(), "b".to_string()];
        assert!(is_tool_allowed("a", Some(&allowed), None));
        assert!(is_tool_allowed("b", Some(&allowed), None));
        assert!(!is_tool_allowed("c", Some(&allowed), None));
    }

    #[test]
    fn test_is_tool_allowed_denies_when_excluded() {
        let excluded = vec!["b".to_string()];
        assert!(is_tool_allowed("a", None, Some(&excluded)));
        assert!(!is_tool_allowed("b", None, Some(&excluded)));
    }

    #[test]
    fn test_is_tool_allowed_denies_when_not_in_allowed_even_if_excluded_empty() {
        let allowed = vec!["a".to_string()];
        let excluded: Vec<String> = Vec::new();
        assert!(is_tool_allowed("a", Some(&allowed), Some(&excluded)));
        assert!(!is_tool_allowed("b", Some(&allowed), Some(&excluded)));
    }

    #[test]
    fn test_is_scope_allowed() {
        let mut rt = ScopeState::new();
        rt.set(SCOPE_ALLOWED_TOOLS_KEY, vec!["a", "b"]).unwrap();
        rt.set(SCOPE_EXCLUDED_TOOLS_KEY, vec!["b"]).unwrap();
        assert!(is_scope_allowed(
            Some(&rt),
            "a",
            SCOPE_ALLOWED_TOOLS_KEY,
            SCOPE_EXCLUDED_TOOLS_KEY
        ));
        assert!(!is_scope_allowed(
            Some(&rt),
            "b",
            SCOPE_ALLOWED_TOOLS_KEY,
            SCOPE_EXCLUDED_TOOLS_KEY
        ));
        assert!(!is_scope_allowed(
            Some(&rt),
            "c",
            SCOPE_ALLOWED_TOOLS_KEY,
            SCOPE_EXCLUDED_TOOLS_KEY
        ));
    }
}
