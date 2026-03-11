use crate::storage::RunOrigin;

/// Error type for typed run configuration mutations.
#[derive(Debug, Clone, thiserror::Error, PartialEq, Eq)]
pub enum RuntimeOptionsError {
    #[error("run config field already set: {field}")]
    AlreadySet { field: &'static str },
}

/// Strongly typed scope policy carried with a resolved run.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ScopePolicy {
    allowed_tools: Option<Vec<String>>,
    excluded_tools: Option<Vec<String>>,
    allowed_skills: Option<Vec<String>>,
    excluded_skills: Option<Vec<String>>,
    allowed_agents: Option<Vec<String>>,
    excluded_agents: Option<Vec<String>>,
}

impl ScopePolicy {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn allowed_tools(&self) -> Option<&[String]> {
        self.allowed_tools.as_deref()
    }

    pub fn excluded_tools(&self) -> Option<&[String]> {
        self.excluded_tools.as_deref()
    }

    pub fn allowed_skills(&self) -> Option<&[String]> {
        self.allowed_skills.as_deref()
    }

    pub fn excluded_skills(&self) -> Option<&[String]> {
        self.excluded_skills.as_deref()
    }

    pub fn allowed_agents(&self) -> Option<&[String]> {
        self.allowed_agents.as_deref()
    }

    pub fn excluded_agents(&self) -> Option<&[String]> {
        self.excluded_agents.as_deref()
    }

    pub fn set_allowed_tools_if_absent(&mut self, values: Option<&[String]>) {
        if self.allowed_tools.is_none() {
            self.allowed_tools = normalize_scope_values(values);
        }
    }

    pub fn set_excluded_tools_if_absent(&mut self, values: Option<&[String]>) {
        if self.excluded_tools.is_none() {
            self.excluded_tools = normalize_scope_values(values);
        }
    }

    pub fn set_allowed_skills_if_absent(&mut self, values: Option<&[String]>) {
        if self.allowed_skills.is_none() {
            self.allowed_skills = normalize_scope_values(values);
        }
    }

    pub fn set_excluded_skills_if_absent(&mut self, values: Option<&[String]>) {
        if self.excluded_skills.is_none() {
            self.excluded_skills = normalize_scope_values(values);
        }
    }

    pub fn set_allowed_agents_if_absent(&mut self, values: Option<&[String]>) {
        if self.allowed_agents.is_none() {
            self.allowed_agents = normalize_scope_values(values);
        }
    }

    pub fn set_excluded_agents_if_absent(&mut self, values: Option<&[String]>) {
        if self.excluded_agents.is_none() {
            self.excluded_agents = normalize_scope_values(values);
        }
    }
}

fn normalize_scope_values(values: Option<&[String]>) -> Option<Vec<String>> {
    let parsed: Vec<String> = values
        .into_iter()
        .flatten()
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .collect();
    if parsed.is_empty() {
        None
    } else {
        Some(parsed)
    }
}

/// Typed per-run configuration shared across runtime contexts.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RuntimeOptions {
    policy: ScopePolicy,
    parent_tool_call_id: Option<String>,
}

impl RuntimeOptions {
    pub const PARENT_TOOL_CALL_ID_FIELD: &'static str = "parent_tool_call_id";

    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn policy(&self) -> &ScopePolicy {
        &self.policy
    }

    pub fn policy_mut(&mut self) -> &mut ScopePolicy {
        &mut self.policy
    }

    pub fn parent_tool_call_id(&self) -> Option<&str> {
        self.parent_tool_call_id.as_deref()
    }

    pub fn set_parent_tool_call_id(
        &mut self,
        value: impl Into<String>,
    ) -> Result<(), RuntimeOptionsError> {
        if self.parent_tool_call_id.is_some() {
            return Err(RuntimeOptionsError::AlreadySet {
                field: Self::PARENT_TOOL_CALL_ID_FIELD,
            });
        }
        let value = value.into();
        if !value.trim().is_empty() {
            self.parent_tool_call_id = Some(value);
        }
        Ok(())
    }
}

/// Strongly typed identity for the active run.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RunExecutionContext {
    pub run_id: String,
    pub parent_run_id: Option<String>,
    pub agent_id: String,
    pub origin: RunOrigin,
    pub parent_tool_call_id: Option<String>,
}

impl RunExecutionContext {
    #[must_use]
    pub const fn new(
        run_id: String,
        parent_run_id: Option<String>,
        agent_id: String,
        origin: RunOrigin,
    ) -> Self {
        Self {
            run_id,
            parent_run_id,
            agent_id,
            origin,
            parent_tool_call_id: None,
        }
    }

    #[must_use]
    pub fn with_parent_tool_call_id(mut self, parent_tool_call_id: impl Into<String>) -> Self {
        let value = parent_tool_call_id.into();
        if !value.trim().is_empty() {
            self.parent_tool_call_id = Some(value);
        }
        self
    }

    pub fn run_id_opt(&self) -> Option<&str> {
        let run_id = self.run_id.trim();
        if run_id.is_empty() {
            None
        } else {
            Some(run_id)
        }
    }

    pub fn parent_run_id_opt(&self) -> Option<&str> {
        self.parent_run_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
    }

    pub fn agent_id_opt(&self) -> Option<&str> {
        let agent_id = self.agent_id.trim();
        if agent_id.is_empty() {
            None
        } else {
            Some(agent_id)
        }
    }

    pub fn parent_tool_call_id_opt(&self) -> Option<&str> {
        self.parent_tool_call_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scope_policy_normalizes_values() {
        let mut policy = ScopePolicy::new();
        policy.set_allowed_tools_if_absent(Some(&[" a ".to_string(), "".to_string()]));
        assert_eq!(policy.allowed_tools(), Some(&["a".to_string()][..]));
    }

    #[test]
    fn runtime_options_parent_tool_call_id_is_set_once() {
        let mut config = RuntimeOptions::new();
        config
            .set_parent_tool_call_id("call-1")
            .expect("first set should succeed");
        let err = config
            .set_parent_tool_call_id("call-2")
            .expect_err("second set should fail");
        assert_eq!(
            err,
            RuntimeOptionsError::AlreadySet {
                field: RuntimeOptions::PARENT_TOOL_CALL_ID_FIELD,
            }
        );
    }
}
