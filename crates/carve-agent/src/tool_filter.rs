use crate::AgentDefinition;

pub(crate) fn is_tool_allowed(
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

pub(crate) fn is_tool_allowed_by_definition(tool_id: &str, def: &AgentDefinition) -> bool {
    is_tool_allowed(
        tool_id,
        def.allowed_tools.as_deref(),
        def.excluded_tools.as_deref(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
