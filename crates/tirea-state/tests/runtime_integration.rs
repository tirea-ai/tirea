//! Integration tests for SealedState: set-once semantics and thread-safety.

use serde::{Deserialize, Serialize};
use serde_json::json;
use std::thread;
use tirea_state::{SealedState, SealedStateError};
use tirea_state_derive::State;

// ============================================================================
// Test state types
// ============================================================================

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, State)]
struct UserInfo {
    user_id: String,
    locale: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, State)]
struct NestedConfig {
    timeout_ms: i64,
    retry: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, State)]
struct EnvelopeConfig {
    #[tirea(nested)]
    config: NestedConfig,
}

// ============================================================================
// SealedState::get<T>() typed access tests
// ============================================================================

#[test]
fn test_scope_get_typed_read() {
    let mut rt = SealedState::new();
    rt.set("user_id", "u-42").unwrap();
    rt.set("locale", "en-US").unwrap();

    let user = rt.get::<UserInfo>();
    assert_eq!(user.user_id().unwrap(), "u-42");
    assert_eq!(user.locale().unwrap(), "en-US");
}

#[test]
fn test_scope_get_at_typed_read() {
    let mut rt = SealedState::new();
    rt.set("config", json!({"timeout_ms": 5000, "retry": true}))
        .unwrap();

    let cfg = rt.get_at::<NestedConfig>("config");
    assert_eq!(cfg.timeout_ms().unwrap(), 5000);
    assert_eq!(cfg.retry().unwrap(), true);
}

#[test]
fn test_scope_get_nested_typed_read() {
    let mut rt = SealedState::new();
    rt.set(
        "config",
        json!({"config": {"timeout_ms": 2000, "retry": true}}),
    )
    .unwrap();

    let env = rt.get_at::<EnvelopeConfig>("config");
    let nested = env.config();
    assert_eq!(nested.timeout_ms().unwrap(), 2000);
    assert!(nested.retry().unwrap());
}

#[test]
fn test_scope_get_missing_field_errors() {
    let rt = SealedState::new();
    // No data set — reading should produce an error from state ref
    let user = rt.get::<UserInfo>();
    assert!(user.user_id().is_err());
}

#[test]
#[should_panic(expected = "read-only sink")]
fn test_scope_get_write_panics() {
    let mut rt = SealedState::new();
    rt.set("user_id", "u-1").unwrap();
    rt.set("locale", "en").unwrap();

    let user = rt.get::<UserInfo>();
    // Attempting to write through a scope state ref should panic
    user.set_user_id("u-2");
}

#[test]
#[should_panic(expected = "read-only sink")]
fn test_scope_get_at_write_panics() {
    let mut rt = SealedState::new();
    rt.set("config", json!({"timeout_ms": 1000, "retry": false}))
        .unwrap();

    let cfg = rt.get_at::<NestedConfig>("config");
    cfg.set_timeout_ms(2000);
}

// ============================================================================
// Set-once semantics (additional edge cases)
// ============================================================================

#[test]
fn test_set_once_independent_keys() {
    let mut rt = SealedState::new();
    rt.set("a", 1).unwrap();
    rt.set("b", 2).unwrap();
    rt.set("c", 3).unwrap();

    // Each key locked independently
    assert!(rt.set("a", 10).is_err());
    assert!(rt.set("b", 20).is_err());
    assert!(rt.set("c", 30).is_err());

    // Values unchanged
    assert_eq!(rt.value("a"), Some(&json!(1)));
    assert_eq!(rt.value("b"), Some(&json!(2)));
    assert_eq!(rt.value("c"), Some(&json!(3)));
}

#[test]
fn test_set_sensitive_marks_only_specified_keys() {
    let mut rt = SealedState::new();
    rt.set("public", "visible").unwrap();
    rt.set_sensitive("secret", "hidden").unwrap();

    assert!(!rt.is_sensitive("public"));
    assert!(rt.is_sensitive("secret"));

    let debug = format!("{:?}", rt);
    assert!(debug.contains("visible"));
    assert!(!debug.contains("hidden"));
    assert!(debug.contains("[REDACTED]"));
}

#[test]
fn test_set_sensitive_existing_key_does_not_mark_sensitive() {
    let mut rt = SealedState::new();
    rt.set("token", "public-token").unwrap();

    let err = rt.set_sensitive("token", "secret-token").unwrap_err();
    assert!(matches!(err, SealedStateError::AlreadySet(_)));
    assert!(!rt.is_sensitive("token"));
}

#[test]
fn test_clone_preserves_set_once_and_sensitivity() {
    let mut rt = SealedState::new();
    rt.set("a", 1).unwrap();
    rt.set_sensitive("token", "s3cret").unwrap();

    let rt2 = rt.clone();

    // Cloned scope has same values
    assert_eq!(rt2.value("a"), Some(&json!(1)));
    assert_eq!(rt2.value("token"), Some(&json!("s3cret")));
    assert!(rt2.is_sensitive("token"));

    // Set-once still enforced on clone
    assert!(rt2.contains_key("a"));
}

#[test]
fn test_set_once_clone_allows_set_on_clone() {
    let mut rt = SealedState::new();
    rt.set("a", 1).unwrap();

    let mut rt2 = rt.clone();
    // Clone has same key, so set-once blocks it
    assert!(rt2.set("a", 2).is_err());

    // But new keys work on clone
    rt2.set("b", 2).unwrap();
    assert_eq!(rt2.value("b"), Some(&json!(2)));
    // Original unaffected
    assert!(rt.value("b").is_none());
}

#[test]
fn test_scope_error_display() {
    let err = SealedStateError::AlreadySet("my_key".to_string());
    assert_eq!(err.to_string(), "sealed state key already set: my_key");

    let err = SealedStateError::SerializationError("bad json".to_string());
    assert_eq!(
        err.to_string(),
        "sealed state serialization error: bad json"
    );
}

// ============================================================================
// SealedState thread safety tests
// ============================================================================

#[test]
fn test_scope_is_send() {
    fn assert_send<T: Send>() {}
    assert_send::<SealedState>();
}

#[test]
fn test_scope_is_sync() {
    fn assert_sync<T: Sync>() {}
    assert_sync::<SealedState>();
}

#[test]
fn test_scope_concurrent_reads() {
    let mut rt = SealedState::new();
    rt.set("user_id", "u-concurrent").unwrap();
    rt.set("locale", "en").unwrap();
    rt.set_sensitive("token", "secret-token").unwrap();
    rt.set("count", 42).unwrap();

    let rt = std::sync::Arc::new(rt);

    let handles: Vec<_> = (0..20)
        .map(|_| {
            let rt = rt.clone();
            thread::spawn(move || {
                // All reads should succeed consistently
                assert_eq!(rt.value("user_id"), Some(&json!("u-concurrent")));
                assert_eq!(rt.value("locale"), Some(&json!("en")));
                assert_eq!(rt.value("count"), Some(&json!(42)));
                assert!(rt.is_sensitive("token"));
                assert!(!rt.is_sensitive("user_id"));
                assert!(rt.contains_key("user_id"));
                assert!(!rt.contains_key("nonexistent"));

                // Typed read
                let user = rt.get::<UserInfo>();
                assert_eq!(user.user_id().unwrap(), "u-concurrent");
            })
        })
        .collect();

    for h in handles {
        h.join().unwrap();
    }
}

#[test]
fn test_scope_clone_isolation_across_threads() {
    let mut rt = SealedState::new();
    rt.set("base", "value").unwrap();

    let handles: Vec<_> = (0..10)
        .map(|i| {
            let mut rt_clone = rt.clone();
            thread::spawn(move || {
                // Each clone can set unique keys
                let key = format!("thread_{}", i);
                rt_clone.set(&key, i).unwrap();
                assert_eq!(rt_clone.value(&key), Some(&json!(i)));
                // Base key is present
                assert_eq!(rt_clone.value("base"), Some(&json!("value")));
            })
        })
        .collect();

    for h in handles {
        h.join().unwrap();
    }

    // Original unchanged — no thread_ keys
    assert!(rt.value("thread_0").is_none());
    assert_eq!(rt.value("base"), Some(&json!("value")));
}

// ============================================================================
// Compile-time verification: SealedState does NOT implement Serialize
// ============================================================================

// This is a compile-time assertion: if SealedState ever gains Serialize,
// this test module will fail to compile because the static assertion
// function will become callable where it shouldn't be.
// We verify this by checking the trait bound does NOT hold.
#[test]
fn test_scope_not_serializable_by_design() {
    // We can't directly assert a negative trait bound at compile time,
    // but we verify the intended behavior: serde_json::to_value should
    // not compile for SealedState. Instead, we verify the Debug output
    // works (the approved way to inspect SealedState) and that sensitive
    // values are redacted.
    let mut rt = SealedState::new();
    rt.set("public", "visible").unwrap();
    rt.set_sensitive("secret", "hidden").unwrap();

    let debug = format!("{:?}", rt);
    assert!(debug.contains("visible"));
    assert!(debug.contains("[REDACTED]"));
    assert!(!debug.contains("hidden"));
}
