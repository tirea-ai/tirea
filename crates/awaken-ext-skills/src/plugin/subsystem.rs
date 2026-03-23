use std::sync::Arc;

use crate::registry::SkillRegistry;

use super::{ActiveSkillInstructionsPlugin, SkillDiscoveryPlugin};

/// High-level facade for wiring skills into an agent.
#[derive(Clone)]
pub struct SkillSubsystem {
    registry: Arc<dyn SkillRegistry>,
}

impl std::fmt::Debug for SkillSubsystem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SkillSubsystem").finish_non_exhaustive()
    }
}

impl SkillSubsystem {
    pub fn new(registry: Arc<dyn SkillRegistry>) -> Self {
        Self { registry }
    }

    pub fn registry(&self) -> &Arc<dyn SkillRegistry> {
        &self.registry
    }

    /// Build the discovery plugin (injects skills catalog before inference and registers skill tools).
    pub fn discovery_plugin(&self) -> SkillDiscoveryPlugin {
        SkillDiscoveryPlugin::new(self.registry.clone())
    }

    /// Build the active instructions plugin (injects active skill instructions).
    pub fn active_instructions_plugin(&self) -> ActiveSkillInstructionsPlugin {
        ActiveSkillInstructionsPlugin::new(self.registry.clone())
    }
}
