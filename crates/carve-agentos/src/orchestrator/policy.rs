use super::AgentDefinition;
use crate::contracts::tool::Tool;
use carve_state::ScopeState;
use std::collections::HashMap;
use std::sync::Arc;

pub(super) use crate::contracts::runtime::{is_id_allowed, is_scope_allowed};
pub(super) use carve_agent_loop::engine::tool_filter::{
    SCOPE_ALLOWED_TOOLS_KEY, SCOPE_EXCLUDED_TOOLS_KEY,
};
pub(super) use carve_agent_extension_skills::{
    SCOPE_ALLOWED_SKILLS_KEY, SCOPE_EXCLUDED_SKILLS_KEY,
};

/// Scope key: delegate-agent allow-list policy.
pub(super) const SCOPE_ALLOWED_AGENTS_KEY: &str = "__agent_policy_allowed_agents";
/// Scope key: delegate-agent deny-list policy.
pub(super) const SCOPE_EXCLUDED_AGENTS_KEY: &str = "__agent_policy_excluded_agents";

pub(super) fn filter_tools_in_place(
    tools: &mut HashMap<String, Arc<dyn Tool>>,
    allowed: Option<&[String]>,
    excluded: Option<&[String]>,
) {
    tools.retain(|id, _| is_id_allowed(id, allowed, excluded));
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
