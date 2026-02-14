//! AG-UI combined interaction plugin.
//!
//! Combines interaction response handling and frontend tool interception
//! to keep AG-UI request wiring as a single plugin unit.

use super::frontend_tool::FrontendToolPlugin;
use super::interaction_response::InteractionResponsePlugin;
use crate::phase::{Phase, StepContext};
use crate::plugin::AgentPlugin;
use async_trait::async_trait;
use std::collections::HashSet;
use carve_state::Context;

/// Combined AG-UI interaction plugin.
///
/// Internally delegates to:
/// - `InteractionResponsePlugin` (response handling)
/// - `FrontendToolPlugin` (frontend tool interception)
///
/// Delegation order is fixed as response → frontend for each phase.
pub struct AgUiInteractionPlugin {
    response: InteractionResponsePlugin,
    frontend: FrontendToolPlugin,
}

impl AgUiInteractionPlugin {
    /// Build combined plugin from explicit frontend tools and responses.
    pub fn new(
        frontend_tools: HashSet<String>,
        approved_ids: Vec<String>,
        denied_ids: Vec<String>,
    ) -> Self {
        Self {
            response: InteractionResponsePlugin::new(approved_ids, denied_ids),
            frontend: FrontendToolPlugin::new(frontend_tools),
        }
    }

    /// Build combined plugin from frontend tools only.
    pub fn with_frontend_tools(frontend_tools: HashSet<String>) -> Self {
        Self::new(frontend_tools, Vec::new(), Vec::new())
    }

    /// Build combined plugin from interaction responses only.
    pub fn with_responses(approved_ids: Vec<String>, denied_ids: Vec<String>) -> Self {
        Self::new(HashSet::new(), approved_ids, denied_ids)
    }

    /// Whether this plugin should be installed for the current request.
    pub fn is_active(&self) -> bool {
        self.response.has_responses() || self.frontend.has_frontend_tools()
    }

    /// Whether request contains frontend tools and therefore needs tool stubs.
    pub fn has_frontend_tools(&self) -> bool {
        self.frontend.has_frontend_tools()
    }

    /// Whether any interaction responses are present.
    pub fn has_responses(&self) -> bool {
        self.response.has_responses()
    }

    /// Check if a specific tool is configured as a frontend tool.
    pub fn is_frontend_tool(&self, tool_name: &str) -> bool {
        self.frontend.is_frontend_tool(tool_name)
    }

    /// Check if an interaction ID is approved.
    pub fn is_approved(&self, interaction_id: &str) -> bool {
        self.response.is_approved(interaction_id)
    }

    /// Check if an interaction ID is denied.
    pub fn is_denied(&self, interaction_id: &str) -> bool {
        self.response.is_denied(interaction_id)
    }
}

#[async_trait]
impl AgentPlugin for AgUiInteractionPlugin {
    fn id(&self) -> &str {
        "agui_interaction"
    }

    async fn on_phase(&self, phase: Phase, step: &mut StepContext<'_>, ctx: &Context<'_>) {
        self.response.on_phase(phase, step, ctx).await;
        self.frontend.on_phase(phase, step, ctx).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plugin_inactive_without_frontend_tools_or_responses() {
        let plugin = AgUiInteractionPlugin::new(HashSet::new(), Vec::new(), Vec::new());
        assert!(!plugin.is_active());
    }

    #[test]
    fn plugin_active_with_frontend_tools() {
        let mut tools = HashSet::new();
        tools.insert("copyToClipboard".to_string());
        let plugin = AgUiInteractionPlugin::new(tools, Vec::new(), Vec::new());
        assert!(plugin.is_active());
        assert!(plugin.has_frontend_tools());
    }

    #[test]
    fn plugin_active_with_responses() {
        let plugin =
            AgUiInteractionPlugin::new(HashSet::new(), vec!["call_1".to_string()], Vec::new());
        assert!(plugin.is_active());
        assert!(plugin.has_responses());
    }
}
