use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Tool permission behavior.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolPermissionBehavior {
    Allow,
    #[default]
    Ask,
    Deny,
}

/// Permission rule subject.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PermissionSubject {
    Tool { tool_id: String },
}

impl PermissionSubject {
    #[must_use]
    pub fn tool(tool_id: impl Into<String>) -> Self {
        Self::Tool {
            tool_id: tool_id.into(),
        }
    }

    #[must_use]
    pub fn key(&self) -> String {
        match self {
            Self::Tool { tool_id } => format!("tool:{tool_id}"),
        }
    }

    #[must_use]
    pub fn matches_tool(&self, tool_id: &str) -> bool {
        matches!(self, Self::Tool { tool_id: id } if id == tool_id)
    }
}

/// Lifetime of a remembered permission rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionRuleScope {
    Once,
    Session,
    #[default]
    Thread,
    Project,
    User,
}

/// Origin of a remembered permission rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionRuleSource {
    System,
    Skill,
    Session,
    User,
    Cli,
    #[default]
    Runtime,
}

/// Declarative permission rule.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PermissionRule {
    pub subject: PermissionSubject,
    pub behavior: ToolPermissionBehavior,
    #[serde(default)]
    pub scope: PermissionRuleScope,
    #[serde(default)]
    pub source: PermissionRuleSource,
}

impl PermissionRule {
    #[must_use]
    pub fn new_tool(tool_id: impl Into<String>, behavior: ToolPermissionBehavior) -> Self {
        Self {
            subject: PermissionSubject::tool(tool_id),
            behavior,
            scope: PermissionRuleScope::Thread,
            source: PermissionRuleSource::Runtime,
        }
    }

    #[must_use]
    pub fn with_scope(mut self, scope: PermissionRuleScope) -> Self {
        self.scope = scope;
        self
    }

    #[must_use]
    pub fn with_source(mut self, source: PermissionRuleSource) -> Self {
        self.source = source;
        self
    }
}

/// Resolved rule set fed into permission strategy evaluation.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PermissionRuleset {
    pub default_behavior: ToolPermissionBehavior,
    pub rules: HashMap<String, PermissionRule>,
}

impl PermissionRuleset {
    #[must_use]
    pub fn rule_for_tool(&self, tool_id: &str) -> Option<&PermissionRule> {
        self.rules
            .values()
            .find(|rule| rule.subject.matches_tool(tool_id))
    }
}

/// Strategy evaluation output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PermissionEvaluation {
    pub subject: PermissionSubject,
    pub behavior: ToolPermissionBehavior,
    pub matched_rule: Option<PermissionRule>,
}
