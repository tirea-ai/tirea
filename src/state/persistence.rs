use std::sync::Arc;

use crate::error::{StateError, UnknownSlotPolicy};

use super::{PersistedState, SlotMap, StateStore};

impl StateStore {
    pub fn export_persisted(&self) -> Result<PersistedState, StateError> {
        let registry = self.registry.lock().expect("registry lock poisoned");
        let state = self.inner.read().expect("state lock poisoned");
        let mut extensions = std::collections::HashMap::new();

        for slot in registry.slots_by_type.values() {
            if !slot.options.persistent {
                continue;
            }

            if let Some(json) = (slot.export)(state.ext.as_ref()).map_err(|err| match err {
                StateError::SlotEncode { key, message } => StateError::SlotEncode { key, message },
                other => StateError::SlotEncode {
                    key: slot.key.clone(),
                    message: other.to_string(),
                },
            })? {
                extensions.insert(slot.key.clone(), json);
            }
        }

        Ok(PersistedState {
            revision: state.revision,
            extensions,
        })
    }

    pub fn restore_persisted(
        &self,
        persisted: PersistedState,
        unknown_policy: UnknownSlotPolicy,
    ) -> Result<(), StateError> {
        let registry = self.registry.lock().expect("registry lock poisoned");
        let mut next_ext = SlotMap::default();

        for (key, json) in persisted.extensions {
            let Some(slot) = registry.slots_by_key.get(&key) else {
                match unknown_policy {
                    UnknownSlotPolicy::Error => return Err(StateError::UnknownSlot { key }),
                    UnknownSlotPolicy::Skip => continue,
                }
            };

            (slot.import)(&mut next_ext, json).map_err(|err| match err {
                StateError::SlotDecode { key, message } => StateError::SlotDecode { key, message },
                other => StateError::SlotDecode {
                    key: slot.key.clone(),
                    message: other.to_string(),
                },
            })?;
        }

        let mut state = self.inner.write().expect("state lock poisoned");
        state.ext = Arc::new(next_ext);
        state.revision = persisted.revision;
        Ok(())
    }
}
