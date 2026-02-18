use carve_state::ScopeState;

use crate::contracts::runtime::{is_id_allowed, is_scope_allowed as runtime_is_scope_allowed};

pub fn is_tool_allowed(
    tool_id: &str,
    allowed: Option<&[String]>,
    excluded: Option<&[String]>,
) -> bool {
    is_id_allowed(tool_id, allowed, excluded)
}

pub fn is_scope_allowed(
    scope: Option<&ScopeState>,
    id: &str,
    allowed_key: &str,
    excluded_key: &str,
) -> bool {
    runtime_is_scope_allowed(scope, id, allowed_key, excluded_key)
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
