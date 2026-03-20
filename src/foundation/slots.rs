use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::hash::{Hash, Hasher};
use std::marker::PhantomData;
use typedmap::clone::SyncCloneBounds;
use typedmap::{TypedMap, TypedMapKey};

use super::{JsonValue, StateError, decode_json, encode_json};

struct ExtensionMarker;

struct SlotKey<K>(PhantomData<fn() -> K>);

impl<K> SlotKey<K> {
    const fn new() -> Self {
        Self(PhantomData)
    }
}

impl<K> Clone for SlotKey<K> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<K> Copy for SlotKey<K> {}

impl<K> PartialEq for SlotKey<K> {
    fn eq(&self, _other: &Self) -> bool {
        true
    }
}

impl<K> Eq for SlotKey<K> {}

impl<K: 'static> Hash for SlotKey<K> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        std::any::TypeId::of::<K>().hash(state);
    }
}

impl<K> TypedMapKey<ExtensionMarker> for SlotKey<K>
where
    K: StateSlot,
{
    type Value = K::Value;
}

pub struct SlotMap {
    values: TypedMap<ExtensionMarker, SyncCloneBounds, SyncCloneBounds>,
}

impl Default for SlotMap {
    fn default() -> Self {
        Self {
            values: TypedMap::new_with_bounds(),
        }
    }
}

impl Clone for SlotMap {
    fn clone(&self) -> Self {
        let mut values = TypedMap::new_with_bounds();
        for entry in self.values.iter() {
            values.insert_key_value(entry.to_owned());
        }
        Self { values }
    }
}

impl SlotMap {
    pub fn contains<K: StateSlot>(&self) -> bool {
        self.values.contains_key(&SlotKey::<K>::new())
    }

    pub fn get<K: StateSlot>(&self) -> Option<&K::Value> {
        self.values.get(&SlotKey::<K>::new())
    }

    pub fn get_mut<K: StateSlot>(&mut self) -> Option<&mut K::Value> {
        self.values.get_mut(&SlotKey::<K>::new())
    }

    pub fn insert<K: StateSlot>(&mut self, value: K::Value) {
        self.values.insert(SlotKey::<K>::new(), value);
    }

    pub fn remove<K: StateSlot>(&mut self) -> Option<K::Value> {
        self.values.remove(&SlotKey::<K>::new())
    }

    pub fn get_or_insert_default<K: StateSlot>(&mut self) -> &mut K::Value {
        if !self.contains::<K>() {
            self.insert::<K>(K::Value::default());
        }

        self.get_mut::<K>()
            .expect("slot value should exist after insertion")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SlotOptions {
    pub persistent: bool,
    pub retain_on_uninstall: bool,
}

impl Default for SlotOptions {
    fn default() -> Self {
        Self {
            persistent: true,
            retain_on_uninstall: false,
        }
    }
}

pub trait StateSlot: 'static + Send + Sync {
    const KEY: &'static str;

    type Value: Clone + Default + Serialize + DeserializeOwned + Send + Sync + 'static;
    type Update: Send + 'static;

    fn apply(value: &mut Self::Value, update: Self::Update);

    fn encode(value: &Self::Value) -> Result<JsonValue, StateError> {
        encode_json(Self::KEY, value)
    }

    fn decode(value: JsonValue) -> Result<Self::Value, StateError> {
        decode_json(Self::KEY, value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Counter;

    impl StateSlot for Counter {
        const KEY: &'static str = "counter";
        type Value = usize;
        type Update = usize;

        fn apply(value: &mut Self::Value, update: Self::Update) {
            *value += update;
        }
    }

    #[test]
    fn slot_map_can_store_and_update_typed_values() {
        let mut slots = SlotMap::default();
        Counter::apply(slots.get_or_insert_default::<Counter>(), 2);
        Counter::apply(slots.get_or_insert_default::<Counter>(), 3);

        assert_eq!(slots.get::<Counter>().copied(), Some(5));
        assert_eq!(slots.remove::<Counter>(), Some(5));
        assert!(!slots.contains::<Counter>());
    }
}
