//! Profile storage types for cross-run, scoped key-value persistence.

use std::fmt;

use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::Value;

use crate::error::StateError;
use crate::model::{JsonValue, decode_json, encode_json};

/// Identifies who owns a profile entry.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ProfileOwner {
    Agent(String),
    System,
}

impl fmt::Display for ProfileOwner {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ProfileOwner::Agent(name) => write!(f, "agent:{name}"),
            ProfileOwner::System => write!(f, "system"),
        }
    }
}

/// A typed key for profile storage.
pub trait ProfileKey: 'static + Send + Sync {
    const KEY: &'static str;
    type Value: Clone + Default + Serialize + DeserializeOwned + Send + Sync + 'static;

    fn encode(value: &Self::Value) -> Result<JsonValue, StateError> {
        encode_json(Self::KEY, value)
    }

    fn decode(value: JsonValue) -> Result<Self::Value, StateError> {
        decode_json(Self::KEY, value)
    }
}

/// A single profile entry stored in the profile store.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProfileEntry {
    pub key: String,
    pub value: Value,
    pub updated_at: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_owner_display() {
        assert_eq!(ProfileOwner::Agent("foo".into()).to_string(), "agent:foo");
        assert_eq!(ProfileOwner::System.to_string(), "system");
    }

    #[test]
    fn profile_owner_equality() {
        let a = ProfileOwner::Agent("x".into());
        let b = ProfileOwner::Agent("x".into());
        let c = ProfileOwner::Agent("y".into());
        assert_eq!(a, b);
        assert_ne!(a, c);
        assert_ne!(ProfileOwner::System, a);
    }

    #[test]
    fn profile_owner_serde_roundtrip() {
        let variants = vec![ProfileOwner::Agent("alice".into()), ProfileOwner::System];
        for owner in variants {
            let json = serde_json::to_string(&owner).expect("serialize");
            let back: ProfileOwner = serde_json::from_str(&json).expect("deserialize");
            assert_eq!(owner, back);
        }
    }

    #[test]
    fn profile_entry_serde_roundtrip() {
        let entry = ProfileEntry {
            key: "lang".into(),
            value: serde_json::json!("en"),
            updated_at: 1234567890,
        };
        let json = serde_json::to_string(&entry).expect("serialize");
        let back: ProfileEntry = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(entry.key, back.key);
        assert_eq!(entry.value, back.value);
        assert_eq!(entry.updated_at, back.updated_at);
    }

    struct Locale;

    impl ProfileKey for Locale {
        const KEY: &'static str = "locale";
        type Value = String;
    }

    #[test]
    fn profile_key_encode_decode_roundtrip() {
        let value = "en-US".to_string();
        let encoded = Locale::encode(&value).expect("encode");
        let decoded = Locale::decode(encoded).expect("decode");
        assert_eq!(value, decoded);
    }

    #[test]
    fn profile_key_decode_missing_returns_default() {
        let empty = serde_json::json!("");
        let decoded = Locale::decode(empty).expect("decode");
        assert_eq!(decoded, String::default());
    }
}
