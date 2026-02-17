use carve_state::ScopeState;
use serde_json::Value;

pub const SCOPE_ALLOWED_SKILLS_KEY: &str = "__agent_policy_allowed_skills";
pub const SCOPE_EXCLUDED_SKILLS_KEY: &str = "__agent_policy_excluded_skills";

fn is_allowed(id: &str, allowed: Option<&[String]>, excluded: Option<&[String]>) -> bool {
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
    is_allowed(id, allowed.as_deref(), excluded.as_deref())
}
