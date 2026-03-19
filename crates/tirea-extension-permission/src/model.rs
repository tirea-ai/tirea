use crate::matcher::{self, MatchResult, Specificity};
use crate::pattern::ToolCallPattern;
use serde::{Deserialize, Serialize};
use serde_json::Value;
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
    /// Legacy: exact tool ID match.
    Tool { tool_id: String },
    /// Pattern-based match: function-call-style glob/regex on tool name and args.
    Pattern { pattern: ToolCallPattern },
}

impl PermissionSubject {
    #[must_use]
    pub fn tool(tool_id: impl Into<String>) -> Self {
        Self::Tool {
            tool_id: tool_id.into(),
        }
    }

    #[must_use]
    pub fn pattern(pattern: ToolCallPattern) -> Self {
        Self::Pattern { pattern }
    }

    #[must_use]
    pub fn key(&self) -> String {
        match self {
            Self::Tool { tool_id } => format!("tool:{tool_id}"),
            Self::Pattern { pattern } => format!("pattern:{pattern}"),
        }
    }

    #[must_use]
    pub fn matches_tool(&self, tool_id: &str) -> bool {
        match self {
            Self::Tool { tool_id: id } => id == tool_id,
            Self::Pattern { pattern } => {
                matcher::pattern_matches(pattern, tool_id, &Value::Null).is_match()
            }
        }
    }

    /// Match against a tool call with arguments. Returns specificity if matched.
    #[must_use]
    pub fn matches_tool_call(&self, tool_id: &str, tool_args: &Value) -> Option<Specificity> {
        match self {
            Self::Tool { tool_id: id } => {
                if id == tool_id {
                    Some(Specificity {
                        tool_kind: 3,
                        has_args: false,
                        field_count: 0,
                        field_precision: 0,
                    })
                } else {
                    None
                }
            }
            Self::Pattern { pattern } => {
                match matcher::pattern_matches(pattern, tool_id, tool_args) {
                    MatchResult::Match { specificity } => Some(specificity),
                    MatchResult::NoMatch => None,
                }
            }
        }
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
    /// Rule originates from `AgentDefinition.permission_rules`.
    Definition,
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

    /// Create a pattern-based rule from a parsed [`ToolCallPattern`].
    #[must_use]
    pub fn new_pattern(pattern: ToolCallPattern, behavior: ToolPermissionBehavior) -> Self {
        Self {
            subject: PermissionSubject::pattern(pattern),
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
    /// Find a matching rule by tool ID only (legacy path).
    #[must_use]
    pub fn rule_for_tool(&self, tool_id: &str) -> Option<&PermissionRule> {
        self.rules
            .values()
            .find(|rule| rule.subject.matches_tool(tool_id))
    }

    /// Find the highest-priority matching rule considering tool args.
    ///
    /// Evaluation order (like firewall rules):
    /// 1. **Deny** — if any deny rule matches, deny immediately.
    /// 2. **Allow** — if any allow rule matches, allow.
    /// 3. **Ask** — remaining unmatched calls fall to ask.
    ///
    /// Within the same behavior tier, higher specificity wins.
    #[must_use]
    pub fn rule_for_tool_call(&self, tool_id: &str, tool_args: &Value) -> Option<&PermissionRule> {
        self.rules
            .values()
            .filter_map(|rule| {
                rule.subject
                    .matches_tool_call(tool_id, tool_args)
                    .map(|specificity| (rule, specificity))
            })
            .max_by(|(a, a_spec), (b, b_spec)| {
                let a_priority = behavior_priority(a.behavior);
                let b_priority = behavior_priority(b.behavior);
                a_priority.cmp(&b_priority).then_with(|| a_spec.cmp(b_spec))
            })
            .map(|(rule, _)| rule)
    }

    /// Collect tool IDs that are unconditionally denied (no arg conditions).
    ///
    /// These tools can be safely excluded from the LLM prompt (visibility
    /// filtering) since no possible arguments would make them pass.
    /// Only exact-name deny rules without arg conditions qualify.
    #[must_use]
    pub fn unconditionally_denied_tools(&self) -> Vec<&str> {
        use crate::pattern::{ArgMatcher, ToolMatcher};

        self.rules
            .values()
            .filter(|rule| rule.behavior == ToolPermissionBehavior::Deny)
            .filter_map(|rule| match &rule.subject {
                // Legacy exact tool deny — always unconditional.
                PermissionSubject::Tool { tool_id } => Some(tool_id.as_str()),
                // Pattern deny with exact tool name and no arg conditions.
                PermissionSubject::Pattern { pattern }
                    if matches!(&pattern.tool, ToolMatcher::Exact(_))
                        && matches!(&pattern.args, ArgMatcher::Any) =>
                {
                    if let ToolMatcher::Exact(name) = &pattern.tool {
                        Some(name.as_str())
                    } else {
                        None
                    }
                }
                _ => None,
            })
            .collect()
    }
}

/// Evaluation priority: Deny > Allow > Ask.
///
/// Deny is evaluated first (highest priority). If no deny matches, explicit
/// allow rules are checked next. Ask is the weakest — it represents the
/// "unmatched" default state where human approval is needed.
fn behavior_priority(behavior: ToolPermissionBehavior) -> u8 {
    match behavior {
        ToolPermissionBehavior::Deny => 2,
        ToolPermissionBehavior::Allow => 1,
        ToolPermissionBehavior::Ask => 0,
    }
}

/// Strategy evaluation output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PermissionEvaluation {
    pub subject: PermissionSubject,
    pub behavior: ToolPermissionBehavior,
    pub matched_rule: Option<PermissionRule>,
}
