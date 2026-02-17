use carve_agent_contract::agent::AgentDefinition;
use carve_state::ScopeState;
use serde_json::Value;

pub(crate) const SCOPE_ALLOWED_TOOLS_KEY: &str = "__agent_policy_allowed_tools";
pub(crate) const SCOPE_EXCLUDED_TOOLS_KEY: &str = "__agent_policy_excluded_tools";
pub(crate) const SCOPE_ALLOWED_SKILLS_KEY: &str = "__agent_policy_allowed_skills";
pub(crate) const SCOPE_EXCLUDED_SKILLS_KEY: &str = "__agent_policy_excluded_skills";
pub(crate) const SCOPE_ALLOWED_AGENTS_KEY: &str = "__agent_policy_allowed_agents";
pub(crate) const SCOPE_EXCLUDED_AGENTS_KEY: &str = "__agent_policy_excluded_agents";

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

pub(crate) fn set_scope_filter_if_absent(
    scope: &mut ScopeState,
    key: &str,
    values: Option<&[String]>,
) -> Result<(), carve_state::ScopeStateError> {
    if scope.value(key).is_some() {
        return Ok(());
    }
    if let Some(values) = values {
        scope.set(key, values.to_vec())?;
    }
    Ok(())
}

pub(crate) fn set_scope_filters_from_definition_if_absent(
    scope: &mut ScopeState,
    def: &AgentDefinition,
) -> Result<(), carve_state::ScopeStateError> {
    set_scope_filter_if_absent(scope, SCOPE_ALLOWED_TOOLS_KEY, def.allowed_tools.as_deref())?;
    set_scope_filter_if_absent(
        scope,
        SCOPE_EXCLUDED_TOOLS_KEY,
        def.excluded_tools.as_deref(),
    )?;
    set_scope_filter_if_absent(
        scope,
        SCOPE_ALLOWED_SKILLS_KEY,
        def.allowed_skills.as_deref(),
    )?;
    set_scope_filter_if_absent(
        scope,
        SCOPE_EXCLUDED_SKILLS_KEY,
        def.excluded_skills.as_deref(),
    )?;
    set_scope_filter_if_absent(
        scope,
        SCOPE_ALLOWED_AGENTS_KEY,
        def.allowed_agents.as_deref(),
    )?;
    set_scope_filter_if_absent(
        scope,
        SCOPE_EXCLUDED_AGENTS_KEY,
        def.excluded_agents.as_deref(),
    )?;
    Ok(())
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

pub(crate) fn is_scope_allowed(
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

    #[test]
    fn test_set_scope_filters_from_definition_if_absent() {
        let mut rt = ScopeState::new();
        let def = AgentDefinition::default()
            .with_allowed_tools(vec!["a".to_string()])
            .with_excluded_tools(vec!["b".to_string()])
            .with_allowed_skills(vec!["s1".to_string()])
            .with_excluded_skills(vec!["s2".to_string()])
            .with_allowed_agents(vec!["agent_a".to_string()])
            .with_excluded_agents(vec!["agent_b".to_string()]);

        set_scope_filters_from_definition_if_absent(&mut rt, &def).unwrap();

        assert_eq!(
            rt.value(SCOPE_ALLOWED_TOOLS_KEY),
            Some(&serde_json::json!(["a"]))
        );
        assert_eq!(
            rt.value(SCOPE_EXCLUDED_TOOLS_KEY),
            Some(&serde_json::json!(["b"]))
        );
        assert_eq!(
            rt.value(SCOPE_ALLOWED_SKILLS_KEY),
            Some(&serde_json::json!(["s1"]))
        );
        assert_eq!(
            rt.value(SCOPE_EXCLUDED_SKILLS_KEY),
            Some(&serde_json::json!(["s2"]))
        );
        assert_eq!(
            rt.value(SCOPE_ALLOWED_AGENTS_KEY),
            Some(&serde_json::json!(["agent_a"]))
        );
        assert_eq!(
            rt.value(SCOPE_EXCLUDED_AGENTS_KEY),
            Some(&serde_json::json!(["agent_b"]))
        );
    }
}
