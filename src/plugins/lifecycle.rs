use crate::error::StateError;
use crate::state::MutationBatch;

use super::{PluginDescriptor, PluginRegistrar};

pub trait Plugin: Send + Sync + 'static {
    fn descriptor(&self) -> PluginDescriptor;

    fn register(&self, _registrar: &mut PluginRegistrar) -> Result<(), StateError> {
        Ok(())
    }

    fn on_install(&self, _patch: &mut MutationBatch) -> Result<(), StateError> {
        Ok(())
    }

    fn on_uninstall(&self, _patch: &mut MutationBatch) -> Result<(), StateError> {
        Ok(())
    }
}
