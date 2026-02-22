//! Unified interaction mechanism plugin.
//!
//! This plugin is strategy-agnostic: it applies static interaction responses.

use super::interaction_response::InteractionResponsePlugin;
use super::INTERACTION_PLUGIN_ID;
use async_trait::async_trait;
use tirea_contract::plugin::phase::{
    AfterInferenceContext, AfterToolExecuteContext, BeforeInferenceContext,
    BeforeToolExecuteContext, RunEndContext, RunStartContext, StepEndContext, StepStartContext,
};
use tirea_contract::plugin::AgentPlugin;
use tirea_contract::InteractionResponse;
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

    /// Build plugin from explicit interaction response payloads.
    pub fn from_interaction_responses(responses: Vec<InteractionResponse>) -> Self {
        Self {
            static_response: InteractionResponsePlugin::from_responses(responses),
        }
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

}

impl Default for InteractionPlugin {
    fn default() -> Self {
        Self::new(Vec::new(), Vec::new())
    }
}

#[async_trait]
impl AgentPlugin for InteractionPlugin {
    fn id(&self) -> &str {
        INTERACTION_PLUGIN_ID
    }

    async fn run_start(&self, ctx: &mut RunStartContext<'_, '_>) {
        self.static_response.run_start(ctx).await;
    }

    async fn step_start(&self, ctx: &mut StepStartContext<'_, '_>) {
        self.static_response.step_start(ctx).await;
    }

    async fn before_inference(&self, ctx: &mut BeforeInferenceContext<'_, '_>) {
        self.static_response.before_inference(ctx).await;
    }

    async fn after_inference(&self, ctx: &mut AfterInferenceContext<'_, '_>) {
        self.static_response.after_inference(ctx).await;
    }

    async fn before_tool_execute(&self, ctx: &mut BeforeToolExecuteContext<'_, '_>) {
        self.static_response.before_tool_execute(ctx).await;
    }

    async fn after_tool_execute(&self, ctx: &mut AfterToolExecuteContext<'_, '_>) {
        self.static_response.after_tool_execute(ctx).await;
    }

    async fn step_end(&self, ctx: &mut StepEndContext<'_, '_>) {
        self.static_response.step_end(ctx).await;
    }

    async fn run_end(&self, ctx: &mut RunEndContext<'_, '_>) {
        self.static_response.run_end(ctx).await;
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
