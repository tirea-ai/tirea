use serde::{Deserialize, Serialize};

use crate::error::StateError;
use crate::state::MutationBatch;

use super::PluginRegistrar;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginMeta {
    pub name: &'static str,
}

pub trait StatePlugin: Send + Sync + 'static {
    fn meta(&self) -> PluginMeta;

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
