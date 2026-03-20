use serde::{Deserialize, Serialize, de::DeserializeOwned};

use crate::error::StateError;
use crate::state::StateSlot;

use super::{JsonValue, decode_json, encode_json};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RuntimeEffect {
    AddUserMessage { message: String },
    AddSystemReminder { message: String },
    Terminate { reason: String },
    Suspend { reason: String },
    Block { reason: String },
    PublishJson { topic: String, payload: JsonValue },
}

impl EffectSpec for RuntimeEffect {
    const KEY: &'static str = "__runtime.runtime_effect";
    type Payload = Self;
}

pub trait EffectSpec: 'static + Send + Sync {
    const KEY: &'static str;

    type Payload: Serialize + DeserializeOwned + Send + Sync + 'static;
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TypedEffect {
    pub key: String,
    pub payload: JsonValue,
}

impl TypedEffect {
    pub fn from_spec<E: EffectSpec>(payload: &E::Payload) -> Result<Self, StateError> {
        Ok(Self {
            key: E::KEY.to_string(),
            payload: encode_json(E::KEY, payload)?,
        })
    }

    pub fn decode<E: EffectSpec>(&self) -> Result<E::Payload, StateError> {
        decode_json(E::KEY, self.payload.clone())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EffectLogEntry {
    pub id: u64,
    pub key: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum EffectLogUpdate {
    Append(EffectLogEntry),
    TrimToLast { keep: usize },
    Clear,
}

pub struct EffectLog;

impl StateSlot for EffectLog {
    const KEY: &'static str = "__runtime.effect_log";

    type Value = Vec<EffectLogEntry>;
    type Update = EffectLogUpdate;

    fn apply(value: &mut Self::Value, update: Self::Update) {
        match update {
            EffectLogUpdate::Append(entry) => value.push(entry),
            EffectLogUpdate::TrimToLast { keep } => trim_to_last(value, keep),
            EffectLogUpdate::Clear => value.clear(),
        }
    }
}

fn trim_to_last<T>(value: &mut Vec<T>, keep: usize) {
    if keep == 0 {
        value.clear();
        return;
    }

    if value.len() > keep {
        let drop_count = value.len() - keep;
        value.drain(0..drop_count);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typed_effect_round_trip_works() {
        let effect = RuntimeEffect::Terminate {
            reason: "done".into(),
        };

        let encoded = TypedEffect::from_spec::<RuntimeEffect>(&effect)
            .expect("runtime effect encoding should succeed");
        let decoded = encoded
            .decode::<RuntimeEffect>()
            .expect("runtime effect decoding should succeed");

        assert_eq!(decoded, effect);
    }
}
