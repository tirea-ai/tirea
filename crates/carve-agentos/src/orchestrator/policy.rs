use super::AgentDefinition;
use crate::contracts::tool::Tool;
use carve_state::ScopeState;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;

pub(super) use crate::contracts::runtime::{
    SCOPE_ALLOWED_AGENTS_KEY, SCOPE_ALLOWED_SKILLS_KEY, SCOPE_ALLOWED_TOOLS_KEY,
    SCOPE_EXCLUDED_AGENTS_KEY, SCOPE_EXCLUDED_SKILLS_KEY, SCOPE_EXCLUDED_TOOLS_KEY,
};

pub(super) fn is_allowed(
    id: &str,
    allowed: Option<&[String]>,
    excluded: Option<&[String]>,
) -> bool {
    if let Some(allowed) = allowed {
        if !allowed.iter().any(|t| t == id) {
            return false;
        }
    }
    if let Some(excluded) = excluded {
        if excluded.iter().any(|t| t == id) {
            return false;
        }
    }
    true
}

pub(super) fn filter_tools_in_place(
    tools: &mut HashMap<String, Arc<dyn Tool>>,
    allowed: Option<&[String]>,
    excluded: Option<&[String]>,
) {
    tools.retain(|id, _| is_allowed(id, allowed, excluded));
}

pub(super) fn set_scope_filter_if_absent(
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

pub(super) fn set_scope_filters_from_definition_if_absent(
    scope: &mut ScopeState,
    definition: &AgentDefinition,
) -> Result<(), carve_state::ScopeStateError> {
    set_scope_filter_if_absent(
        scope,
        SCOPE_ALLOWED_TOOLS_KEY,
        definition.allowed_tools.as_deref(),
    )?;
    set_scope_filter_if_absent(
        scope,
        SCOPE_EXCLUDED_TOOLS_KEY,
        definition.excluded_tools.as_deref(),
    )?;
    set_scope_filter_if_absent(
        scope,
        SCOPE_ALLOWED_SKILLS_KEY,
        definition.allowed_skills.as_deref(),
    )?;
    set_scope_filter_if_absent(
        scope,
        SCOPE_EXCLUDED_SKILLS_KEY,
        definition.excluded_skills.as_deref(),
    )?;
    set_scope_filter_if_absent(
        scope,
        SCOPE_ALLOWED_AGENTS_KEY,
        definition.allowed_agents.as_deref(),
    )?;
    set_scope_filter_if_absent(
        scope,
        SCOPE_EXCLUDED_AGENTS_KEY,
        definition.excluded_agents.as_deref(),
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

pub(super) fn is_scope_allowed(
    scope: Option<&ScopeState>,
    id: &str,
    allowed_key: &str,
    excluded_key: &str,
) -> bool {
    let allowed = parse_scope_filter(scope.and_then(|s| s.value(allowed_key)));
    let excluded = parse_scope_filter(scope.and_then(|s| s.value(excluded_key)));
    is_allowed(id, allowed.as_deref(), excluded.as_deref())
}
