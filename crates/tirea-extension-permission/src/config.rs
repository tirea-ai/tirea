use crate::model::PermissionRule;
use std::sync::Arc;

/// Permission rules carried in `AgentRunConfig.extensions`.
///
/// NOT persisted to thread state — lives only for the run lifetime.
/// Populated during resolve from `AgentDefinition.permission_rules`.
/// The plugin merges these with thread-state rules at evaluation time.
#[derive(Debug, Clone)]
pub struct PermissionRulesConfig {
    rules: Arc<[PermissionRule]>,
}

impl PermissionRulesConfig {
    pub fn new(rules: Vec<PermissionRule>) -> Self {
        Self {
            rules: Arc::from(rules),
        }
    }

    pub fn rules(&self) -> &[PermissionRule] {
        &self.rules
    }
}
