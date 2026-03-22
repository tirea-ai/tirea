use crate::error::StateError;
use crate::registry::spec::AgentSpec;
use crate::state::MutationBatch;

use super::{PluginDescriptor, PluginRegistrar};

/// Schema declaration for a plugin's config section.
///
/// Returned by [`Plugin::config_schemas`] to enable eager validation
/// during resolve — before hooks ever run.
pub struct ConfigSchema {
    /// Section key in `AgentSpec.sections` (must match `PluginConfigKey::KEY`).
    pub key: &'static str,
    /// Validator: attempts to deserialize the JSON value into the expected type.
    /// Returns `Ok(())` if valid, `Err` with a serde error message if not.
    pub validate: fn(&serde_json::Value) -> Result<(), serde_json::Error>,
}

pub trait Plugin: Send + Sync + 'static {
    fn descriptor(&self) -> PluginDescriptor;

    /// Declare capabilities: state keys, hooks, action handlers, effect handlers, permission checkers.
    /// Called once per resolve to build the ExecutionEnv.
    fn register(&self, _registrar: &mut PluginRegistrar) -> Result<(), StateError> {
        Ok(())
    }

    /// Declare config section schemas for eager validation during resolve.
    ///
    /// Override this to declare which `AgentSpec.sections` keys this plugin
    /// owns and how to validate them. The resolve pipeline calls this to
    /// catch malformed config before hooks run.
    fn config_schemas(&self) -> Vec<ConfigSchema> {
        Vec::new()
    }

    /// Agent activated: read spec config, write initial state.
    /// Called when this plugin becomes active for a specific agent.
    fn on_activate(
        &self,
        _agent_spec: &AgentSpec,
        _patch: &mut MutationBatch,
    ) -> Result<(), StateError> {
        Ok(())
    }

    /// Agent deactivated: clean up agent-scoped state.
    /// Called when switching away from an agent that uses this plugin.
    fn on_deactivate(&self, _patch: &mut MutationBatch) -> Result<(), StateError> {
        Ok(())
    }
}
