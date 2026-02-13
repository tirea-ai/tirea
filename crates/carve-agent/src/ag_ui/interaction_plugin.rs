//! AG-UI combined interaction plugin.
//!
//! Combines interaction response handling and frontend tool interception
//! to keep AG-UI request wiring as a single plugin unit.

use super::{FrontendToolPlugin, InteractionResponsePlugin, RunAgentRequest};
use crate::phase::{Phase, StepContext};
use crate::plugin::AgentPlugin;
use async_trait::async_trait;

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
    /// Build combined plugin from request payload.
    pub fn from_request(request: &RunAgentRequest) -> Self {
        Self {
            response: InteractionResponsePlugin::from_request(request),
            frontend: FrontendToolPlugin::from_request(request),
        }
    }

    /// Whether this plugin should be installed for the current request.
    pub fn is_active(&self) -> bool {
        self.response.has_responses() || self.frontend.has_frontend_tools()
    }

    /// Whether request contains frontend tools and therefore needs tool stubs.
    pub fn has_frontend_tools(&self) -> bool {
        self.frontend.has_frontend_tools()
    }
}

#[async_trait]
impl AgentPlugin for AgUiInteractionPlugin {
    fn id(&self) -> &str {
        "agui_interaction"
    }

    async fn on_phase(&self, phase: Phase, step: &mut StepContext<'_>) {
        self.response.on_phase(phase, step).await;
        self.frontend.on_phase(phase, step).await;
    }
}
