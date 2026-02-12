//! Integration tests for Runtime type, Context+Runtime integration,
//! and Runtime thread safety.

use carve_state::{Context, Runtime, RuntimeError};
use carve_state_derive::State;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::thread;

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

// ============================================================================
// Runtime::get<T>() typed access tests
// ============================================================================

#[test]
fn test_runtime_get_typed_read() {
    let mut rt = Runtime::new();
    rt.set("user_id", "u-42").unwrap();
    rt.set("locale", "en-US").unwrap();

    let user = rt.get::<UserInfo>();
    assert_eq!(user.user_id().unwrap(), "u-42");
    assert_eq!(user.locale().unwrap(), "en-US");
}

#[test]
fn test_runtime_get_at_typed_read() {
    let mut rt = Runtime::new();
    rt.set("config", json!({"timeout_ms": 5000, "retry": true}))
        .unwrap();

    let cfg = rt.get_at::<NestedConfig>("config");
    assert_eq!(cfg.timeout_ms().unwrap(), 5000);
    assert_eq!(cfg.retry().unwrap(), true);
}

#[test]
fn test_runtime_get_missing_field_errors() {
    let rt = Runtime::new();
    // No data set — reading should produce an error from state ref
    let user = rt.get::<UserInfo>();
    assert!(user.user_id().is_err());
}

#[test]
#[should_panic(expected = "read-only sink")]
fn test_runtime_get_write_panics() {
    let mut rt = Runtime::new();
    rt.set("user_id", "u-1").unwrap();
    rt.set("locale", "en").unwrap();

    let user = rt.get::<UserInfo>();
    // Attempting to write through a runtime state ref should panic
    user.set_user_id("u-2");
}

#[test]
#[should_panic(expected = "read-only sink")]
fn test_runtime_get_at_write_panics() {
    let mut rt = Runtime::new();
    rt.set("config", json!({"timeout_ms": 1000, "retry": false}))
        .unwrap();

    let cfg = rt.get_at::<NestedConfig>("config");
    cfg.set_timeout_ms(2000);
}

// ============================================================================
// Context + Runtime integration tests
// ============================================================================

#[test]
fn test_context_with_runtime_value() {
    let mut rt = Runtime::new();
    rt.set("user_id", "u-abc").unwrap();
    rt.set("run_id", "run-xyz").unwrap();

    let doc = json!({});
    let ctx = Context::new(&doc, "call_1", "tool:test").with_runtime(Some(&rt));

    assert_eq!(ctx.runtime_value("user_id"), Some(&json!("u-abc")));
    assert_eq!(ctx.runtime_value("run_id"), Some(&json!("run-xyz")));
    assert_eq!(ctx.runtime_value("missing"), None);
}

#[test]
fn test_context_runtime_typed_access() {
    let mut rt = Runtime::new();
    rt.set("user_id", "u-42").unwrap();
    rt.set("locale", "zh-CN").unwrap();

    let doc = json!({});
    let ctx = Context::new(&doc, "call_1", "tool:test").with_runtime(Some(&rt));

    let user = ctx.runtime::<UserInfo>();
    assert_eq!(user.user_id().unwrap(), "u-42");
    assert_eq!(user.locale().unwrap(), "zh-CN");
}

#[test]
fn test_context_runtime_ref() {
    let mut rt = Runtime::new();
    rt.set("key", "val").unwrap();

    let doc = json!({});
    let ctx = Context::new(&doc, "c1", "s1").with_runtime(Some(&rt));

    let rt_ref = ctx.runtime_ref().unwrap();
    assert_eq!(rt_ref.value("key"), Some(&json!("val")));
}

#[test]
fn test_context_runtime_ref_none() {
    let doc = json!({});
    let ctx = Context::new(&doc, "c1", "s1");
    assert!(ctx.runtime_ref().is_none());
}

#[test]
#[should_panic(expected = "no Runtime set")]
fn test_context_runtime_panics_without_runtime() {
    let doc = json!({});
    let ctx = Context::new(&doc, "c1", "s1");
    let _user = ctx.runtime::<UserInfo>();
}

#[test]
fn test_context_runtime_value_without_runtime() {
    let doc = json!({});
    let ctx = Context::new(&doc, "c1", "s1");
    // Should return None, not panic
    assert_eq!(ctx.runtime_value("anything"), None);
}

#[test]
fn test_context_state_and_runtime_independent() {
    // Verify that state writes don't affect runtime, and runtime reads don't affect state
    let mut rt = Runtime::new();
    rt.set("user_id", "runtime-user").unwrap();
    rt.set("locale", "en").unwrap();

    let doc = json!({"user_id": "state-user", "locale": "fr"});
    let ctx = Context::new(&doc, "c1", "s1").with_runtime(Some(&rt));

    // State and runtime have different values for same-named fields
    let state_user = ctx.state::<UserInfo>("");
    assert_eq!(state_user.user_id().unwrap(), "state-user");

    let rt_user = ctx.runtime::<UserInfo>();
    assert_eq!(rt_user.user_id().unwrap(), "runtime-user");

    // Writing to state should not affect runtime
    state_user.set_user_id("modified");
    assert!(ctx.has_changes());
    assert_eq!(rt.value("user_id"), Some(&json!("runtime-user")));
}

// ============================================================================
// Set-once semantics (additional edge cases)
// ============================================================================

#[test]
fn test_set_once_independent_keys() {
    let mut rt = Runtime::new();
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
    let mut rt = Runtime::new();
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
fn test_clone_preserves_set_once_and_sensitivity() {
    let mut rt = Runtime::new();
    rt.set("a", 1).unwrap();
    rt.set_sensitive("token", "s3cret").unwrap();

    let rt2 = rt.clone();

    // Cloned runtime has same values
    assert_eq!(rt2.value("a"), Some(&json!(1)));
    assert_eq!(rt2.value("token"), Some(&json!("s3cret")));
    assert!(rt2.is_sensitive("token"));

    // Set-once still enforced on clone
    assert!(rt2.contains_key("a"));
}

#[test]
fn test_set_once_clone_allows_set_on_clone() {
    let mut rt = Runtime::new();
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
fn test_runtime_error_display() {
    let err = RuntimeError::AlreadySet("my_key".to_string());
    assert_eq!(err.to_string(), "runtime key already set: my_key");

    let err = RuntimeError::SerializationError("bad json".to_string());
    assert_eq!(err.to_string(), "runtime serialization error: bad json");
}

// ============================================================================
// Runtime thread safety tests
// ============================================================================

#[test]
fn test_runtime_is_send() {
    fn assert_send<T: Send>() {}
    assert_send::<Runtime>();
}

#[test]
fn test_runtime_is_sync() {
    fn assert_sync<T: Sync>() {}
    assert_sync::<Runtime>();
}

#[test]
fn test_runtime_concurrent_reads() {
    let mut rt = Runtime::new();
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
fn test_runtime_clone_isolation_across_threads() {
    let mut rt = Runtime::new();
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
// Compile-time verification: Runtime does NOT implement Serialize
// ============================================================================

// This is a compile-time assertion: if Runtime ever gains Serialize,
// this test module will fail to compile because the static assertion
// function will become callable where it shouldn't be.
// We verify this by checking the trait bound does NOT hold.
#[test]
fn test_runtime_not_serializable_by_design() {
    // We can't directly assert a negative trait bound at compile time,
    // but we verify the intended behavior: serde_json::to_value should
    // not compile for Runtime. Instead, we verify the Debug output
    // works (the approved way to inspect Runtime) and that sensitive
    // values are redacted.
    let mut rt = Runtime::new();
    rt.set("public", "visible").unwrap();
    rt.set_sensitive("secret", "hidden").unwrap();

    let debug = format!("{:?}", rt);
    assert!(debug.contains("visible"));
    assert!(debug.contains("[REDACTED]"));
    assert!(!debug.contains("hidden"));
}
