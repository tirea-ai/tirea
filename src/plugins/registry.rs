use std::any::TypeId;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use crate::error::StateError;
use crate::model::JsonValue;
use crate::state::{SlotMap, SlotOptions, StateSlot};

use super::StatePlugin;

#[derive(Clone)]
pub(crate) struct SlotRegistration {
    pub(crate) type_id: TypeId,
    pub(crate) key: String,
    pub(crate) options: SlotOptions,
    pub(crate) export: fn(&SlotMap) -> Result<Option<JsonValue>, StateError>,
    pub(crate) import: fn(&mut SlotMap, JsonValue) -> Result<(), StateError>,
    pub(crate) clear: fn(&mut SlotMap),
}

impl SlotRegistration {
    pub(crate) fn new<K: StateSlot>(options: SlotOptions) -> Self {
        Self {
            type_id: TypeId::of::<K>(),
            key: K::KEY.into(),
            options,
            export: |map| match map.get::<K>() {
                Some(value) => K::encode(value).map(Some),
                None => Ok(None),
            },
            import: |map, json| {
                let value = K::decode(json)?;
                map.insert::<K>(value);
                Ok(())
            },
            clear: |map| {
                let _ = map.remove::<K>();
            },
        }
    }
}

#[derive(Default)]
pub struct PluginRegistry {
    pub(crate) plugins: HashMap<TypeId, InstalledPlugin>,
    pub(crate) slots_by_type: HashMap<TypeId, SlotRegistration>,
    pub(crate) slots_by_key: HashMap<String, SlotRegistration>,
}

pub struct InstalledPlugin {
    pub(crate) plugin: Arc<dyn StatePlugin>,
    pub(crate) owned_slot_type_ids: Vec<TypeId>,
}

impl PluginRegistry {
    pub(crate) fn ensure_slot(&self, key: &str) -> Result<(), StateError> {
        if self.slots_by_key.contains_key(key) {
            Ok(())
        } else {
            Err(StateError::UnknownSlot { key: key.into() })
        }
    }
}

pub struct PluginRegistrar {
    pub(crate) slots: Vec<SlotRegistration>,
    slot_type_ids: HashSet<TypeId>,
    slot_keys: HashSet<String>,
}

impl PluginRegistrar {
    pub(crate) fn new() -> Self {
        Self {
            slots: Vec::new(),
            slot_type_ids: HashSet::new(),
            slot_keys: HashSet::new(),
        }
    }

    pub fn register_slot<K>(&mut self, options: SlotOptions) -> Result<(), StateError>
    where
        K: StateSlot,
    {
        let type_id = TypeId::of::<K>();
        if !self.slot_type_ids.insert(type_id) || !self.slot_keys.insert(K::KEY.to_string()) {
            return Err(StateError::SlotAlreadyRegistered {
                key: K::KEY.to_string(),
            });
        }

        self.slots.push(SlotRegistration::new::<K>(options));
        Ok(())
    }
}
