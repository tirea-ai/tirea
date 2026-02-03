//! Context data extension for generic key-value storage.
//!
//! Provides methods for storing and retrieving arbitrary data:
//! - `set(key, value)` - Store a value
//! - `get(key)` - Retrieve a value
//!
//! This is useful for passing data between tools and plugins without
//! defining custom state types.
//!
//! # Example
//!
//! ```ignore
//! use carve_agent::prelude::*;
//!
//! async fn execute(&self, args: Value, ctx: &Context<'_>) -> Result<ToolResult, ToolError> {
//!     // Store working directory
//!     ctx.set("working_dir", json!("/tmp/project"));
//!
//!     // Later, retrieve it
//!     if let Some(dir) = ctx.get("working_dir") {
//!         ctx.add_reminder(format!("Working in: {}", dir));
//!     }
//!
//!     Ok(ToolResult::success("tool", json!({})))
//! }
//! ```

use carve_state::Context;
use carve_state_derive::State;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

/// State path for context data.
pub const CONTEXT_DATA_STATE_PATH: &str = "context";

/// Context data state stored in session state.
#[derive(Debug, Clone, Default, Serialize, Deserialize, State)]
pub struct ContextDataState {
    /// Generic key-value store.
    #[serde(default)]
    #[carve(default = "HashMap::new()")]
    pub data: HashMap<String, Value>,
}

/// Extension trait for context data on Context.
pub trait ContextDataExt {
    /// Set a value in the context data store.
    fn set(&self, key: &str, value: Value);

    /// Get a value from the context data store.
    fn get(&self, key: &str) -> Option<Value>;

    /// Check if a key exists in the context data store.
    fn has(&self, key: &str) -> bool;

    /// Remove a value from the context data store.
    fn remove(&self, key: &str);

    /// Get all context data keys.
    fn keys(&self) -> Vec<String>;
}

impl ContextDataExt for Context<'_> {
    fn set(&self, key: &str, value: Value) {
        let state = self.state::<ContextDataState>(CONTEXT_DATA_STATE_PATH);
        state.data_insert(key.to_string(), value);
    }

    fn get(&self, key: &str) -> Option<Value> {
        let state = self.state::<ContextDataState>(CONTEXT_DATA_STATE_PATH);
        state.data().ok().and_then(|data| data.get(key).cloned())
    }

    fn has(&self, key: &str) -> bool {
        self.get(key).is_some()
    }

    fn remove(&self, key: &str) {
        let state = self.state::<ContextDataState>(CONTEXT_DATA_STATE_PATH);
        // Read current data, remove the key, and set it back
        if let Ok(mut data) = state.data() {
            data.remove(key);
            state.set_data(data);
        }
    }

    fn keys(&self) -> Vec<String> {
        let state = self.state::<ContextDataState>(CONTEXT_DATA_STATE_PATH);
        state
            .data()
            .ok()
            .map(|data| data.keys().cloned().collect())
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_context_data_state_default() {
        let state = ContextDataState::default();
        assert!(state.data.is_empty());
    }

    #[test]
    fn test_context_data_state_serialization() {
        let mut state = ContextDataState::default();
        state.data.insert("key1".to_string(), json!("value1"));
        state.data.insert("key2".to_string(), json!(42));

        let json = serde_json::to_string(&state).unwrap();
        let parsed: ContextDataState = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.data.len(), 2);
        assert_eq!(parsed.data.get("key1"), Some(&json!("value1")));
    }

    #[test]
    fn test_set_and_get() {
        let doc = json!({
            "context": { "data": {} }
        });
        let ctx = Context::new(&doc, "call_1", "test");

        ctx.set("key", json!("value"));
        assert!(ctx.has_changes());

        // Note: We can't verify get() returns the new value because
        // the patch hasn't been applied yet. This tests the write path.
    }

    #[test]
    fn test_get_existing() {
        let doc = json!({
            "context": {
                "data": {
                    "existing_key": "existing_value"
                }
            }
        });
        let ctx = Context::new(&doc, "call_1", "test");

        let value = ctx.get("existing_key");
        assert_eq!(value, Some(json!("existing_value")));
    }

    #[test]
    fn test_get_nonexistent() {
        let doc = json!({
            "context": { "data": {} }
        });
        let ctx = Context::new(&doc, "call_1", "test");

        let value = ctx.get("nonexistent");
        assert!(value.is_none());
    }

    #[test]
    fn test_has() {
        let doc = json!({
            "context": {
                "data": {
                    "exists": true
                }
            }
        });
        let ctx = Context::new(&doc, "call_1", "test");

        assert!(ctx.has("exists"));
        assert!(!ctx.has("does_not_exist"));
    }

    #[test]
    fn test_remove() {
        let doc = json!({
            "context": {
                "data": {
                    "to_remove": "value"
                }
            }
        });
        let ctx = Context::new(&doc, "call_1", "test");

        ctx.remove("to_remove");
        assert!(ctx.has_changes());
    }

    #[test]
    fn test_keys() {
        let doc = json!({
            "context": {
                "data": {
                    "key1": "value1",
                    "key2": "value2",
                    "key3": "value3"
                }
            }
        });
        let ctx = Context::new(&doc, "call_1", "test");

        let keys = ctx.keys();
        assert_eq!(keys.len(), 3);
        assert!(keys.contains(&"key1".to_string()));
        assert!(keys.contains(&"key2".to_string()));
        assert!(keys.contains(&"key3".to_string()));
    }

    #[test]
    fn test_keys_empty() {
        let doc = json!({
            "context": { "data": {} }
        });
        let ctx = Context::new(&doc, "call_1", "test");

        let keys = ctx.keys();
        assert!(keys.is_empty());
    }

    #[test]
    fn test_complex_value() {
        let doc = json!({
            "context": { "data": {} }
        });
        let ctx = Context::new(&doc, "call_1", "test");

        let complex = json!({
            "nested": {
                "array": [1, 2, 3],
                "object": { "key": "value" }
            }
        });

        ctx.set("complex", complex.clone());
        assert!(ctx.has_changes());
    }
}
