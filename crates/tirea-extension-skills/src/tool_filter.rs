use tirea_contract::runtime::is_scope_allowed as runtime_is_scope_allowed;
use tirea_contract::RunConfig;

pub const SCOPE_ALLOWED_SKILLS_KEY: &str = "__agent_policy_allowed_skills";
pub const SCOPE_EXCLUDED_SKILLS_KEY: &str = "__agent_policy_excluded_skills";

pub fn is_scope_allowed(
    scope: Option<&RunConfig>,
    id: &str,
    allowed_key: &str,
    excluded_key: &str,
) -> bool {
    runtime_is_scope_allowed(scope, id, allowed_key, excluded_key)
}
