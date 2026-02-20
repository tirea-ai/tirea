use crate::RunConfig;
use serde_json::Value;

/// Check whether an identifier is allowed by optional allow/deny lists.
#[must_use]
pub fn is_id_allowed(id: &str, allowed: Option<&[String]>, excluded: Option<&[String]>) -> bool {
    if let Some(allowed) = allowed {
        if !allowed.iter().any(|v| v == id) {
            return false;
        }
    }
    if let Some(excluded) = excluded {
        if excluded.iter().any(|v| v == id) {
            return false;
        }
    }
    true
}

/// Parse scope filter values from a JSON array into normalized string values.
#[must_use]
pub fn parse_scope_filter(values: Option<&Value>) -> Option<Vec<String>> {
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

/// Check whether an identifier is allowed by filters stored in scope keys.
#[must_use]
pub fn is_scope_allowed(
    run_config: Option<&RunConfig>,
    id: &str,
    allowed_key: &str,
    excluded_key: &str,
) -> bool {
    let allowed = parse_scope_filter(run_config.and_then(|s| s.value(allowed_key)));
    let excluded = parse_scope_filter(run_config.and_then(|s| s.value(excluded_key)));
    is_id_allowed(id, allowed.as_deref(), excluded.as_deref())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parse_scope_filter_normalizes_strings() {
        let parsed = parse_scope_filter(Some(&json!([" a ", "", "b", 1, null]))).unwrap();
        assert_eq!(parsed, vec!["a".to_string(), "b".to_string()]);
    }

    #[test]
    fn is_id_allowed_uses_allow_and_exclude_lists() {
        let allowed = vec!["a".to_string(), "b".to_string()];
        let excluded = vec!["b".to_string()];
        assert!(is_id_allowed("a", Some(&allowed), Some(&excluded)));
        assert!(!is_id_allowed("b", Some(&allowed), Some(&excluded)));
        assert!(!is_id_allowed("c", Some(&allowed), Some(&excluded)));
    }

    #[test]
    fn is_scope_allowed_reads_filters_from_scope() {
        let allowed_key = "__test_allowed";
        let excluded_key = "__test_excluded";
        let mut scope = RunConfig::new();
        scope
            .set(allowed_key, vec!["a", "b"])
            .expect("set allowed");
        scope
            .set(excluded_key, vec!["b"])
            .expect("set excluded");

        assert!(is_scope_allowed(
            Some(&scope),
            "a",
            allowed_key,
            excluded_key,
        ));
        assert!(!is_scope_allowed(
            Some(&scope),
            "b",
            allowed_key,
            excluded_key,
        ));
    }
}
