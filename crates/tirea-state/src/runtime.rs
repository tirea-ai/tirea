//! Sealed state: an ephemeral, non-persistent key/value container.
//!
//! `SealedState` stores data with set-once semantics — each key can be
//! written exactly once, then becomes immutable for the container's lifetime.
//!
//! Sensitive keys are tracked separately and redacted in `Debug` output.
//! `Serialize` is intentionally **not** implemented to prevent accidental persistence.

use crate::state::{PatchSink, State};
use crate::{get_at_path, parse_path, DocCell, Path};
use serde_json::Value;
use std::collections::HashSet;
use thiserror::Error;

/// Errors from `SealedState` operations.
#[derive(Debug, Error)]
pub enum SealedStateError {
    /// Attempted to set a key that was already set.
    #[error("sealed state key already set: {0}")]
    AlreadySet(String),
    /// JSON serialization failed.
    #[error("sealed state serialization error: {0}")]
    SerializationError(String),
}

/// Sealed key/value container with set-once semantics.
///
/// Each key can be written exactly once via `put_once()` or `put_sensitive_once()`.
/// After that, the key is immutable for the container's lifetime.
///
/// Consumers generally receive `&SealedState` (read-only), so the Rust borrow checker
/// guarantees no writes occur during execution.
///
/// # Sensitive keys
///
/// Keys set via `set_sensitive()` are redacted in `Debug` output.
/// Use this for tokens, secrets, and other credentials.
///
/// # No `Serialize`
///
/// `SealedState` intentionally does **not** implement `Serialize`,
/// preventing accidental persistence. This is enforced at compile time.
#[derive(Clone)]
pub struct SealedState {
    doc: Value,
    doc_cell: DocCell,
    sensitive_keys: HashSet<String>,
}

impl SealedState {
    /// Create an empty sealed state.
    pub fn new() -> Self {
        let doc = Value::Object(Default::default());
        Self {
            doc_cell: DocCell::new(doc.clone()),
            doc,
            sensitive_keys: HashSet::new(),
        }
    }

    /// Set a key once. Returns error if key already exists.
    pub fn put_once(
        &mut self,
        key: impl Into<String>,
        value: impl serde::Serialize,
    ) -> Result<(), SealedStateError> {
        let key = key.into();
        let obj = self
            .doc
            .as_object_mut()
            .expect("sealed state doc is object");
        if obj.contains_key(&key) {
            return Err(SealedStateError::AlreadySet(key));
        }
        let v = serde_json::to_value(value)
            .map_err(|e| SealedStateError::SerializationError(e.to_string()))?;
        obj.insert(key, v);
        // Keep doc_cell in sync for State trait reads
        *self.doc_cell.get() = self.doc.clone();
        Ok(())
    }

    /// Set a sensitive key once (redacted in `Debug`).
    pub fn put_sensitive_once(
        &mut self,
        key: impl Into<String>,
        value: impl serde::Serialize,
    ) -> Result<(), SealedStateError> {
        let key = key.into();
        self.put_once(key.clone(), value)?;
        self.sensitive_keys.insert(key);
        Ok(())
    }

    /// Backward-compatible alias for `put_once`.
    pub fn set(
        &mut self,
        key: impl Into<String>,
        value: impl serde::Serialize,
    ) -> Result<(), SealedStateError> {
        self.put_once(key, value)
    }

    /// Backward-compatible alias for `put_sensitive_once`.
    pub fn set_sensitive(
        &mut self,
        key: impl Into<String>,
        value: impl serde::Serialize,
    ) -> Result<(), SealedStateError> {
        self.put_sensitive_once(key, value)
    }

    /// Get a typed state reference (same API as `ctx.state::<T>()`).
    ///
    /// Returns a read-only `StateRef` backed by the sealed document.
    /// Any write through this ref will panic (read-only sink).
    pub fn get<T: State>(&self) -> T::Ref<'_> {
        T::state_ref(&self.doc_cell, Path::root(), PatchSink::read_only())
    }

    /// Get a typed state reference at a dot-separated path.
    pub fn get_at<T: State>(&self, path: &str) -> T::Ref<'_> {
        let base = parse_path(path);
        T::state_ref(&self.doc_cell, base, PatchSink::read_only())
    }

    /// Get a raw JSON value by key.
    pub fn value(&self, key: &str) -> Option<&Value> {
        self.doc.as_object().and_then(|obj| obj.get(key))
    }

    /// Get a raw JSON value at a dot-separated path.
    pub fn value_at(&self, path: &str) -> Option<&Value> {
        if path.is_empty() {
            return Some(&self.doc);
        }
        let p = parse_path(path);
        get_at_path(&self.doc, &p)
    }

    /// Check if a key is marked sensitive.
    pub fn is_sensitive(&self, key: &str) -> bool {
        self.sensitive_keys.contains(key)
    }

    /// Check if the container has a key.
    pub fn contains_key(&self, key: &str) -> bool {
        self.doc
            .as_object()
            .map_or(false, |obj| obj.contains_key(key))
    }
}

impl Default for SealedState {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for SealedState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut map = f.debug_map();
        if let Some(obj) = self.doc.as_object() {
            for (k, v) in obj {
                if self.sensitive_keys.contains(k) {
                    map.entry(k, &"[REDACTED]");
                } else {
                    map.entry(k, v);
                }
            }
        }
        map.finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_scope_new_is_empty() {
        let rt = SealedState::new();
        assert!(!rt.contains_key("anything"));
        assert!(rt.value("anything").is_none());
    }

    #[test]
    fn test_scope_default() {
        let rt = SealedState::default();
        assert!(!rt.contains_key("x"));
    }

    #[test]
    fn test_set_once_success() {
        let mut rt = SealedState::new();
        rt.set("user_id", "u123").unwrap();
        assert_eq!(rt.value("user_id"), Some(&json!("u123")));
    }

    #[test]
    fn test_set_once_duplicate_fails() {
        let mut rt = SealedState::new();
        rt.set("key", "first").unwrap();
        let err = rt.set("key", "second").unwrap_err();
        assert!(matches!(err, SealedStateError::AlreadySet(k) if k == "key"));
        // Value unchanged
        assert_eq!(rt.value("key"), Some(&json!("first")));
    }

    #[test]
    fn test_set_sensitive() {
        let mut rt = SealedState::new();
        rt.set_sensitive("token", "secret-abc").unwrap();
        assert!(rt.is_sensitive("token"));
        assert_eq!(rt.value("token"), Some(&json!("secret-abc")));
    }

    #[test]
    fn test_set_sensitive_duplicate_fails() {
        let mut rt = SealedState::new();
        rt.set_sensitive("token", "first").unwrap();
        let err = rt.set_sensitive("token", "second").unwrap_err();
        assert!(matches!(err, SealedStateError::AlreadySet(_)));
    }

    #[test]
    fn test_non_sensitive_key() {
        let rt = SealedState::new();
        assert!(!rt.is_sensitive("anything"));
    }

    #[test]
    fn test_debug_redacts_sensitive() {
        let mut rt = SealedState::new();
        rt.set("user_id", "u123").unwrap();
        rt.set_sensitive("token", "secret").unwrap();

        let debug = format!("{:?}", rt);
        assert!(debug.contains("u123"));
        assert!(debug.contains("[REDACTED]"));
        assert!(!debug.contains("secret"));
    }

    #[test]
    fn test_clone() {
        let mut rt = SealedState::new();
        rt.set("a", 1).unwrap();
        rt.set_sensitive("b", "secret").unwrap();

        let rt2 = rt.clone();
        assert_eq!(rt2.value("a"), Some(&json!(1)));
        assert!(rt2.is_sensitive("b"));
    }

    #[test]
    fn test_value_at_path() {
        let mut rt = SealedState::new();
        rt.set("config", json!({"nested": {"value": 42}})).unwrap();
        assert_eq!(rt.value_at("config.nested.value"), Some(&json!(42)));
        assert_eq!(rt.value_at("config.missing"), None);
    }

    #[test]
    fn test_value_at_empty_path() {
        let mut rt = SealedState::new();
        rt.set("key", "val").unwrap();
        // Empty path returns root doc
        let root = rt.value_at("").unwrap();
        assert!(root.is_object());
    }

    #[test]
    fn test_set_various_types() {
        let mut rt = SealedState::new();
        rt.set("string", "hello").unwrap();
        rt.set("number", 42).unwrap();
        rt.set("bool", true).unwrap();
        rt.set("array", vec![1, 2, 3]).unwrap();

        assert_eq!(rt.value("string"), Some(&json!("hello")));
        assert_eq!(rt.value("number"), Some(&json!(42)));
        assert_eq!(rt.value("bool"), Some(&json!(true)));
        assert_eq!(rt.value("array"), Some(&json!([1, 2, 3])));
    }

    #[test]
    fn test_contains_key() {
        let mut rt = SealedState::new();
        assert!(!rt.contains_key("x"));
        rt.set("x", 1).unwrap();
        assert!(rt.contains_key("x"));
    }
}
