use crate::runtime::loop_runner::AgentDefinition;
use carve_state::Runtime;
use serde_json::Value;

pub(crate) const RUNTIME_ALLOWED_TOOLS_KEY: &str = "__agent_policy_allowed_tools";
pub(crate) const RUNTIME_EXCLUDED_TOOLS_KEY: &str = "__agent_policy_excluded_tools";
pub(crate) const RUNTIME_ALLOWED_SKILLS_KEY: &str = "__agent_policy_allowed_skills";
pub(crate) const RUNTIME_EXCLUDED_SKILLS_KEY: &str = "__agent_policy_excluded_skills";
pub(crate) const RUNTIME_ALLOWED_AGENTS_KEY: &str = "__agent_policy_allowed_agents";
pub(crate) const RUNTIME_EXCLUDED_AGENTS_KEY: &str = "__agent_policy_excluded_agents";

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

pub(crate) fn set_runtime_filter_if_absent(
    runtime: &mut Runtime,
    key: &str,
    values: Option<&[String]>,
) -> Result<(), carve_state::RuntimeError> {
    if runtime.value(key).is_some() {
        return Ok(());
    }
    if let Some(values) = values {
        runtime.set(key, values.to_vec())?;
    }
    Ok(())
}

pub(crate) fn set_runtime_filters_from_definition_if_absent(
    runtime: &mut Runtime,
    def: &AgentDefinition,
) -> Result<(), carve_state::RuntimeError> {
    set_runtime_filter_if_absent(
        runtime,
        RUNTIME_ALLOWED_TOOLS_KEY,
        def.allowed_tools.as_deref(),
    )?;
    set_runtime_filter_if_absent(
        runtime,
        RUNTIME_EXCLUDED_TOOLS_KEY,
        def.excluded_tools.as_deref(),
    )?;
    set_runtime_filter_if_absent(
        runtime,
        RUNTIME_ALLOWED_SKILLS_KEY,
        def.allowed_skills.as_deref(),
    )?;
    set_runtime_filter_if_absent(
        runtime,
        RUNTIME_EXCLUDED_SKILLS_KEY,
        def.excluded_skills.as_deref(),
    )?;
    set_runtime_filter_if_absent(
        runtime,
        RUNTIME_ALLOWED_AGENTS_KEY,
        def.allowed_agents.as_deref(),
    )?;
    set_runtime_filter_if_absent(
        runtime,
        RUNTIME_EXCLUDED_AGENTS_KEY,
        def.excluded_agents.as_deref(),
    )?;
    Ok(())
}

fn parse_runtime_filter(values: Option<&Value>) -> Option<Vec<String>> {
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

pub(crate) fn is_runtime_allowed(
    runtime: Option<&Runtime>,
    id: &str,
    allowed_key: &str,
    excluded_key: &str,
) -> bool {
    let allowed = parse_runtime_filter(runtime.and_then(|rt| rt.value(allowed_key)));
    let excluded = parse_runtime_filter(runtime.and_then(|rt| rt.value(excluded_key)));
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
    fn test_is_runtime_allowed() {
        let mut rt = Runtime::new();
        rt.set(RUNTIME_ALLOWED_TOOLS_KEY, vec!["a", "b"]).unwrap();
        rt.set(RUNTIME_EXCLUDED_TOOLS_KEY, vec!["b"]).unwrap();
        assert!(is_runtime_allowed(
            Some(&rt),
            "a",
            RUNTIME_ALLOWED_TOOLS_KEY,
            RUNTIME_EXCLUDED_TOOLS_KEY
        ));
        assert!(!is_runtime_allowed(
            Some(&rt),
            "b",
            RUNTIME_ALLOWED_TOOLS_KEY,
            RUNTIME_EXCLUDED_TOOLS_KEY
        ));
        assert!(!is_runtime_allowed(
            Some(&rt),
            "c",
            RUNTIME_ALLOWED_TOOLS_KEY,
            RUNTIME_EXCLUDED_TOOLS_KEY
        ));
    }

    #[test]
    fn test_set_runtime_filters_from_definition_if_absent() {
        let mut rt = Runtime::new();
        let def = AgentDefinition::default()
            .with_allowed_tools(vec!["a".to_string()])
            .with_excluded_tools(vec!["b".to_string()])
            .with_allowed_skills(vec!["s1".to_string()])
            .with_excluded_skills(vec!["s2".to_string()])
            .with_allowed_agents(vec!["agent_a".to_string()])
            .with_excluded_agents(vec!["agent_b".to_string()]);

        set_runtime_filters_from_definition_if_absent(&mut rt, &def).unwrap();

        assert_eq!(
            rt.value(RUNTIME_ALLOWED_TOOLS_KEY),
            Some(&serde_json::json!(["a"]))
        );
        assert_eq!(
            rt.value(RUNTIME_EXCLUDED_TOOLS_KEY),
            Some(&serde_json::json!(["b"]))
        );
        assert_eq!(
            rt.value(RUNTIME_ALLOWED_SKILLS_KEY),
            Some(&serde_json::json!(["s1"]))
        );
        assert_eq!(
            rt.value(RUNTIME_EXCLUDED_SKILLS_KEY),
            Some(&serde_json::json!(["s2"]))
        );
        assert_eq!(
            rt.value(RUNTIME_ALLOWED_AGENTS_KEY),
            Some(&serde_json::json!(["agent_a"]))
        );
        assert_eq!(
            rt.value(RUNTIME_EXCLUDED_AGENTS_KEY),
            Some(&serde_json::json!(["agent_b"]))
        );
    }
}
