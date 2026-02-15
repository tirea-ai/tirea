//! Unified interaction mechanism plugin.
//!
//! This plugin is strategy-agnostic: it applies static interaction responses.

use super::interaction_response::InteractionResponsePlugin;
use crate::phase::{Phase, StepContext};
use crate::plugin::AgentPlugin;
use async_trait::async_trait;
use carve_state::Context;

/// Unified interaction mechanism plugin.
pub struct InteractionPlugin {
    static_response: InteractionResponsePlugin,
}

impl InteractionPlugin {
    /// Build plugin from interaction responses.
    pub fn new(approved_ids: Vec<String>, denied_ids: Vec<String>) -> Self {
        Self {
            static_response: InteractionResponsePlugin::new(approved_ids, denied_ids),
        }
    }

    /// Build combined plugin from interaction responses only.
    pub fn with_responses(approved_ids: Vec<String>, denied_ids: Vec<String>) -> Self {
        Self::new(approved_ids, denied_ids)
    }

    /// Whether this plugin should be installed for the current request.
    pub fn is_active(&self) -> bool {
        self.static_response.has_responses()
    }

    /// Whether any interaction responses are present.
    pub fn has_responses(&self) -> bool {
        self.static_response.has_responses()
    }

    /// Check if an interaction ID is approved.
    pub fn is_approved(&self, interaction_id: &str) -> bool {
        self.static_response.is_approved(interaction_id)
    }

    /// Check if an interaction ID is denied.
    pub fn is_denied(&self, interaction_id: &str) -> bool {
        self.static_response.is_denied(interaction_id)
    }

    fn response_plugin(&self) -> InteractionResponsePlugin {
        InteractionResponsePlugin::from_responses(self.static_response.responses())
    }
}

impl Default for InteractionPlugin {
    fn default() -> Self {
        Self::new(Vec::new(), Vec::new())
    }
}

#[async_trait]
impl AgentPlugin for InteractionPlugin {
    fn id(&self) -> &str {
        "interaction"
    }

    async fn on_phase(&self, phase: Phase, step: &mut StepContext<'_>, ctx: &Context<'_>) {
        let response = self.response_plugin();
        response.on_phase(phase, step, ctx).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plugin_inactive_without_responses() {
        let plugin = InteractionPlugin::new(Vec::new(), Vec::new());
        assert!(!plugin.is_active());
    }

    #[test]
    fn plugin_active_with_responses() {
        let plugin = InteractionPlugin::new(vec!["call_1".to_string()], Vec::new());
        assert!(plugin.is_active());
        assert!(plugin.has_responses());
    }
}
