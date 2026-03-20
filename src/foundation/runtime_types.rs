use serde::{Deserialize, Serialize, de::DeserializeOwned};

use super::{JsonValue, StateError, StateSlot, decode_json, encode_json};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    RunStart,
    BeforeInference,
    AfterInference,
    BeforeToolExecute,
    AfterToolExecute,
    RunEnd,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScheduledAction {
    pub phase: Phase,
    pub key: String,
    pub payload: JsonValue,
}

impl ScheduledAction {
    pub fn new(phase: Phase, key: impl Into<String>, payload: JsonValue) -> Self {
        Self {
            phase,
            key: key.into(),
            payload,
        }
    }
}

pub trait ScheduledActionSpec: 'static + Send + Sync {
    const KEY: &'static str;
    const PHASE: Phase;

    type Payload: Serialize + DeserializeOwned + Send + Sync + 'static;

    fn encode_payload(payload: &Self::Payload) -> Result<JsonValue, StateError> {
        encode_json(Self::KEY, payload)
    }

    fn decode_payload(payload: JsonValue) -> Result<Self::Payload, StateError> {
        decode_json(Self::KEY, payload)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScheduledActionEnvelope {
    pub id: u64,
    pub action: ScheduledAction,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FailedScheduledAction {
    pub id: u64,
    pub action: ScheduledAction,
    pub error: String,
}

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
#[serde(tag = "op", rename_all = "snake_case")]
pub enum ScheduledActionQueueUpdate {
    Push(ScheduledActionEnvelope),
    Remove { id: u64 },
}

pub struct PendingScheduledActions;

impl StateSlot for PendingScheduledActions {
    const KEY: &'static str = "__runtime.pending_scheduled_actions";

    type Value = Vec<ScheduledActionEnvelope>;
    type Update = ScheduledActionQueueUpdate;

    fn apply(value: &mut Self::Value, update: Self::Update) {
        match update {
            ScheduledActionQueueUpdate::Push(entry) => value.push(entry),
            ScheduledActionQueueUpdate::Remove { id } => value.retain(|entry| entry.id != id),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum FailedScheduledActionUpdate {
    Push(FailedScheduledAction),
    Remove { id: u64 },
}

pub struct FailedScheduledActions;

impl StateSlot for FailedScheduledActions {
    const KEY: &'static str = "__runtime.failed_scheduled_actions";

    type Value = Vec<FailedScheduledAction>;
    type Update = FailedScheduledActionUpdate;

    fn apply(value: &mut Self::Value, update: Self::Update) {
        match update {
            FailedScheduledActionUpdate::Push(entry) => value.push(entry),
            FailedScheduledActionUpdate::Remove { id } => value.retain(|entry| entry.id != id),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScheduledActionLogEntry {
    pub id: u64,
    pub phase: Phase,
    pub key: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum ScheduledActionLogUpdate {
    Append(ScheduledActionLogEntry),
    TrimToLast { keep: usize },
    Clear,
}

pub struct ScheduledActionLog;

impl StateSlot for ScheduledActionLog {
    const KEY: &'static str = "__runtime.scheduled_action_log";

    type Value = Vec<ScheduledActionLogEntry>;
    type Update = ScheduledActionLogUpdate;

    fn apply(value: &mut Self::Value, update: Self::Update) {
        match update {
            ScheduledActionLogUpdate::Append(entry) => value.push(entry),
            ScheduledActionLogUpdate::TrimToLast { keep } => trim_to_last(value, keep),
            ScheduledActionLogUpdate::Clear => value.clear(),
        }
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

    struct TestAction;

    impl ScheduledActionSpec for TestAction {
        const KEY: &'static str = "test.action";
        const PHASE: Phase = Phase::BeforeInference;
        type Payload = String;
    }

    #[test]
    fn scheduled_action_spec_round_trip_works() {
        let payload = "hello".to_string();
        let encoded = TestAction::encode_payload(&payload).expect("encode should succeed");
        let decoded = TestAction::decode_payload(encoded).expect("decode should succeed");

        assert_eq!(decoded, payload);
    }

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

    #[test]
    fn trim_to_last_keeps_latest_entries() {
        let mut values = vec![1, 2, 3, 4];
        trim_to_last(&mut values, 2);
        assert_eq!(values, vec![3, 4]);

        trim_to_last(&mut values, 0);
        assert!(values.is_empty());
    }
}
