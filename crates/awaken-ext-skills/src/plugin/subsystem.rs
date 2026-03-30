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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::InMemorySkillRegistry;

    fn empty_registry() -> Arc<dyn crate::registry::SkillRegistry> {
        Arc::new(InMemorySkillRegistry::new())
    }

    #[test]
    fn subsystem_new_and_registry_accessor() {
        let reg = empty_registry();
        let sub = SkillSubsystem::new(reg.clone());
        assert_eq!(sub.registry().len(), 0);
    }

    #[test]
    fn subsystem_discovery_plugin_returns_plugin() {
        let sub = SkillSubsystem::new(empty_registry());
        let _plugin = sub.discovery_plugin();
    }

    #[test]
    fn subsystem_active_instructions_plugin_returns_plugin() {
        let sub = SkillSubsystem::new(empty_registry());
        let _plugin = sub.active_instructions_plugin();
    }

    #[test]
    fn subsystem_debug_format() {
        let sub = SkillSubsystem::new(empty_registry());
        let debug = format!("{:?}", sub);
        assert!(debug.contains("SkillSubsystem"));
    }
}
