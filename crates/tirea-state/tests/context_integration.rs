//! Integration tests for StateContext and automatic patch collection.
//!
//! These tests verify the core API design where developers use typed state references
//! through StateContext, and all operations are automatically collected.

use tirea_state::{apply_patch, DocCell, State as StateTrait, StateContext};
use tirea_state_derive::State;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::BTreeMap;

// ============================================================================
// Test state types
// ============================================================================

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, State)]
struct CounterState {
    value: i64,
    label: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, State)]
struct TodoState {
    items: Vec<String>,
    completed: Vec<bool>,
    count: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, State)]
struct UserState {
    name: String,
    email: Option<String>,
    age: i64,
    tags: Vec<String>,
    metadata: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, State)]
struct ProfileState {
    bio: String,
    avatar_url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, State)]
struct AccountState {
    username: String,
    #[tirea(nested)]
    profile: ProfileState,
}

// ============================================================================
// Context basic tests
// ============================================================================

#[test]
fn test_context_creation() {
    let doc = json!({"value": 10, "label": "test"});
    let doc_cell = DocCell::new(doc.clone());
    let ctx = StateContext::new(&doc_cell);

    assert!(!ctx.has_changes());
    assert_eq!(ctx.ops_count(), 0);
}

#[test]
fn test_context_state_read() {
    let doc = json!({
        "counters": {
            "main": {
                "value": 42,
                "label": "Main Counter"
            }
        }
    });
    let doc_cell = DocCell::new(doc.clone());
    let ctx = StateContext::new(&doc_cell);

    let counter = ctx.state::<CounterState>("counters.main");
    assert_eq!(counter.value().unwrap(), 42);
    assert_eq!(counter.label().unwrap(), "Main Counter");

    // Reading should not produce any changes
    assert!(!ctx.has_changes());
}

#[test]
fn test_context_state_write() {
    let doc = json!({
        "counters": {
            "main": {
                "value": 10,
                "label": "Counter"
            }
        }
    });
    let doc_cell = DocCell::new(doc.clone());
    let ctx = StateContext::new(&doc_cell);

    let counter = ctx.state::<CounterState>("counters.main");
    counter.set_value(20);
    counter.set_label("Updated Counter");

    assert!(ctx.has_changes());
    assert_eq!(ctx.ops_count(), 2);

    // Take patch and verify
    let tracked = ctx.take_patch();
    assert_eq!(tracked.len(), 2);

    // Apply and verify
    let new_doc = apply_patch(&doc, &tracked).unwrap();
    assert_eq!(new_doc["counters"]["main"]["value"], 20);
    assert_eq!(new_doc["counters"]["main"]["label"], "Updated Counter");
}

#[test]
fn test_context_state_increment() {
    let doc = json!({
        "counter": {
            "value": 100,
            "label": "Test"
        }
    });
    let doc_cell = DocCell::new(doc.clone());
    let ctx = StateContext::new(&doc_cell);

    let counter = ctx.state::<CounterState>("counter");
    counter.increment_value(5);

    let tracked = ctx.take_patch();
    let new_doc = apply_patch(&doc, &tracked).unwrap();
    assert_eq!(new_doc["counter"]["value"], 105);
}

#[test]
fn test_context_state_decrement() {
    let doc = json!({
        "counter": {
            "value": 100,
            "label": "Test"
        }
    });
    let doc_cell = DocCell::new(doc.clone());
    let ctx = StateContext::new(&doc_cell);

    let counter = ctx.state::<CounterState>("counter");
    counter.decrement_value(30);

    let tracked = ctx.take_patch();
    let new_doc = apply_patch(&doc, &tracked).unwrap();
    assert_eq!(new_doc["counter"]["value"], 70);
}

#[test]
fn test_context_state_delete() {
    let doc = json!({
        "counter": {
            "value": 100,
            "label": "Test"
        }
    });
    let doc_cell = DocCell::new(doc.clone());
    let ctx = StateContext::new(&doc_cell);

    let counter = ctx.state::<CounterState>("counter");
    counter.delete_label();

    let tracked = ctx.take_patch();
    let new_doc = apply_patch(&doc, &tracked).unwrap();
    assert!(new_doc["counter"].get("label").is_none());
    assert_eq!(new_doc["counter"]["value"], 100);
}

#[test]
fn test_context_take_patch_clears_ops() {
    let doc = json!({"counter": {"value": 10, "label": "X"}});
    let doc_cell = DocCell::new(doc.clone());
    let ctx = StateContext::new(&doc_cell);

    let counter = ctx.state::<CounterState>("counter");
    counter.set_value(20);

    assert_eq!(ctx.ops_count(), 1);

    let _patch1 = ctx.take_patch();
    assert_eq!(ctx.ops_count(), 0);
    assert!(!ctx.has_changes());

    // Subsequent take returns empty patch
    let patch2 = ctx.take_patch();
    assert!(patch2.is_empty());
}

// ============================================================================
// StateContext with call-like path tests
// ============================================================================

#[test]
fn test_context_call_state() {
    let doc = json!({
        "tool_calls": {
            "call_123": {
                "value": 5,
                "label": "Call State"
            }
        }
    });
    let doc_cell = DocCell::new(doc.clone());
    let ctx = StateContext::new(&doc_cell);

    let state = ctx.state::<CounterState>("tool_calls.call_123");
    assert_eq!(state.value().unwrap(), 5);
    assert_eq!(state.label().unwrap(), "Call State");

    state.set_value(10);

    let tracked = ctx.take_patch();
    let new_doc = apply_patch(&doc, &tracked).unwrap();
    assert_eq!(new_doc["tool_calls"]["call_123"]["value"], 10);
}

// ============================================================================
// Multiple state references tests
// ============================================================================

#[test]
fn test_context_multiple_state_refs() {
    let doc = json!({
        "users": {
            "alice": {"name": "Alice", "email": null, "age": 30, "tags": [], "metadata": {}},
            "bob": {"name": "Bob", "email": "bob@example.com", "age": 25, "tags": [], "metadata": {}}
        }
    });
    let doc_cell = DocCell::new(doc.clone());
    let ctx = StateContext::new(&doc_cell);

    // Get multiple state references
    let alice = ctx.state::<UserState>("users.alice");
    let bob = ctx.state::<UserState>("users.bob");

    // Modify both
    alice.set_age(31);
    alice.set_email(Some("alice@example.com".to_string()));
    bob.increment_age(1);

    let tracked = ctx.take_patch();
    assert_eq!(tracked.len(), 3);

    let new_doc = apply_patch(&doc, &tracked).unwrap();
    assert_eq!(new_doc["users"]["alice"]["age"], 31);
    assert_eq!(new_doc["users"]["alice"]["email"], "alice@example.com");
    assert_eq!(new_doc["users"]["bob"]["age"], 26);
}

#[test]
fn test_context_state_ref_reuse() {
    let doc = json!({"counter": {"value": 0, "label": "X"}});
    let doc_cell = DocCell::new(doc.clone());
    let ctx = StateContext::new(&doc_cell);

    // Get state ref multiple times - all should share the same sink
    let counter1 = ctx.state::<CounterState>("counter");
    counter1.set_value(10);

    let counter2 = ctx.state::<CounterState>("counter");
    counter2.set_value(20);

    let counter3 = ctx.state::<CounterState>("counter");
    counter3.set_label("Updated");

    // All operations collected
    assert_eq!(ctx.ops_count(), 3);
}

// ============================================================================
// Vec and Map operations tests
// ============================================================================

#[test]
fn test_context_vec_operations() {
    let doc = json!({
        "todos": {
            "items": ["Task 1"],
            "completed": [false],
            "count": 1
        }
    });
    let doc_cell = DocCell::new(doc.clone());
    let ctx = StateContext::new(&doc_cell);

    let todos = ctx.state::<TodoState>("todos");
    todos.items_push("Task 2");
    todos.items_push("Task 3");
    todos.completed_push(false);
    todos.completed_push(false);
    todos.increment_count(2);

    let tracked = ctx.take_patch();
    let new_doc = apply_patch(&doc, &tracked).unwrap();

    assert_eq!(
        new_doc["todos"]["items"],
        json!(["Task 1", "Task 2", "Task 3"])
    );
    assert_eq!(new_doc["todos"]["completed"], json!([false, false, false]));
    assert_eq!(new_doc["todos"]["count"], 3);
}

#[test]
fn test_context_map_operations() {
    let doc = json!({
        "user": {
            "name": "Alice",
            "email": null,
            "age": 30,
            "tags": [],
            "metadata": {"key1": "value1"}
        }
    });
    let doc_cell = DocCell::new(doc.clone());
    let ctx = StateContext::new(&doc_cell);

    let user = ctx.state::<UserState>("user");
    user.metadata_insert("key2", "value2");
    user.metadata_insert("key3", "value3");
    user.tags_push("vip");

    let tracked = ctx.take_patch();
    let new_doc = apply_patch(&doc, &tracked).unwrap();

    assert_eq!(new_doc["user"]["metadata"]["key1"], "value1");
    assert_eq!(new_doc["user"]["metadata"]["key2"], "value2");
    assert_eq!(new_doc["user"]["metadata"]["key3"], "value3");
    assert_eq!(new_doc["user"]["tags"], json!(["vip"]));
}

// ============================================================================
// Nested state tests
// ============================================================================

#[test]
fn test_context_nested_state_read() {
    let doc = json!({
        "account": {
            "username": "alice",
            "profile": {
                "bio": "Hello world",
                "avatar_url": "https://example.com/avatar.png"
            }
        }
    });
    let doc_cell = DocCell::new(doc.clone());
    let ctx = StateContext::new(&doc_cell);

    let account = ctx.state::<AccountState>("account");
    assert_eq!(account.username().unwrap(), "alice");

    let profile = account.profile();
    assert_eq!(profile.bio().unwrap(), "Hello world");
    assert_eq!(
        profile.avatar_url().unwrap(),
        Some("https://example.com/avatar.png".to_string())
    );
}

#[test]
fn test_context_nested_state_write() {
    let doc = json!({
        "account": {
            "username": "alice",
            "profile": {
                "bio": "Old bio",
                "avatar_url": null
            }
        }
    });
    let doc_cell = DocCell::new(doc.clone());
    let ctx = StateContext::new(&doc_cell);

    let account = ctx.state::<AccountState>("account");
    account.set_username("alice_updated");
    account.profile().set_bio("New bio");
    account
        .profile()
        .set_avatar_url(Some("https://example.com/new.png".to_string()));

    let tracked = ctx.take_patch();
    let new_doc = apply_patch(&doc, &tracked).unwrap();

    assert_eq!(new_doc["account"]["username"], "alice_updated");
    assert_eq!(new_doc["account"]["profile"]["bio"], "New bio");
    assert_eq!(
        new_doc["account"]["profile"]["avatar_url"],
        "https://example.com/new.png"
    );
}

// ============================================================================
// Option field tests
// ============================================================================

#[test]
fn test_context_option_field_some_to_none() {
    let doc = json!({
        "user": {
            "name": "Alice",
            "email": "alice@example.com",
            "age": 30,
            "tags": [],
            "metadata": {}
        }
    });
    let doc_cell = DocCell::new(doc.clone());
    let ctx = StateContext::new(&doc_cell);

    let user = ctx.state::<UserState>("user");
    user.email_none();

    let tracked = ctx.take_patch();
    let new_doc = apply_patch(&doc, &tracked).unwrap();

    assert!(new_doc["user"]["email"].is_null());
}

#[test]
fn test_context_option_field_none_to_some() {
    let doc = json!({
        "user": {
            "name": "Alice",
            "email": null,
            "age": 30,
            "tags": [],
            "metadata": {}
        }
    });
    let doc_cell = DocCell::new(doc.clone());
    let ctx = StateContext::new(&doc_cell);

    let user = ctx.state::<UserState>("user");
    user.set_email(Some("alice@example.com".to_string()));

    let tracked = ctx.take_patch();
    let new_doc = apply_patch(&doc, &tracked).unwrap();

    assert_eq!(new_doc["user"]["email"], "alice@example.com");
}

// ============================================================================
// Root state tests
// ============================================================================

#[test]
fn test_context_root_state() {
    let doc = json!({
        "value": 100,
        "label": "Root Counter"
    });
    let doc_cell = DocCell::new(doc.clone());
    let ctx = StateContext::new(&doc_cell);

    // Empty path means root
    let counter = ctx.state::<CounterState>("");
    assert_eq!(counter.value().unwrap(), 100);
    assert_eq!(counter.label().unwrap(), "Root Counter");

    counter.set_value(200);

    let tracked = ctx.take_patch();
    let new_doc = apply_patch(&doc, &tracked).unwrap();
    assert_eq!(new_doc["value"], 200);
}

// ============================================================================
// Complex workflow tests
// ============================================================================

#[test]
fn test_context_complex_workflow() {
    // Simulate a realistic tool execution workflow
    let doc = json!({
        "execution": {
            "status": "running",
            "step": 0
        },
        "tool_calls": {
            "call_abc": {
                "value": 0,
                "label": "Processing"
            }
        },
        "results": {
            "items": [],
            "completed": [],
            "count": 0
        }
    });
    let doc_cell = DocCell::new(doc.clone());
    let ctx = StateContext::new(&doc_cell);

    // Read current state
    let call_state = ctx.state::<CounterState>("tool_calls.call_abc");
    let current_step = call_state.value().unwrap();

    // Update call state
    call_state.set_value(current_step + 1);
    call_state.set_label("Step 1 complete");

    // Update results
    let results = ctx.state::<TodoState>("results");
    results.items_push("Result 1");
    results.completed_push(true);
    results.increment_count(1);

    // Verify changes
    let tracked = ctx.take_patch();
    assert_eq!(tracked.len(), 5);

    let new_doc = apply_patch(&doc, &tracked).unwrap();
    assert_eq!(new_doc["tool_calls"]["call_abc"]["value"], 1);
    assert_eq!(
        new_doc["tool_calls"]["call_abc"]["label"],
        "Step 1 complete"
    );
    assert_eq!(new_doc["results"]["items"], json!(["Result 1"]));
    assert_eq!(new_doc["results"]["count"], 1);
}

// ============================================================================
// Error handling tests
// ============================================================================

#[test]
fn test_context_read_missing_field() {
    let doc = json!({
        "counter": {
            "value": 10
            // label is missing
        }
    });
    let doc_cell = DocCell::new(doc.clone());
    let ctx = StateContext::new(&doc_cell);

    let counter = ctx.state::<CounterState>("counter");
    assert_eq!(counter.value().unwrap(), 10);

    // Missing field should error
    let result = counter.label();
    assert!(result.is_err());
}

#[test]
fn test_context_read_missing_path() {
    let doc = json!({
        "other": {}
    });
    let doc_cell = DocCell::new(doc.clone());
    let ctx = StateContext::new(&doc_cell);

    let counter = ctx.state::<CounterState>("counter");

    // Reading from non-existent path should error
    let result = counter.value();
    assert!(result.is_err());
}

// ============================================================================
// Immutability guarantee tests
// ============================================================================

#[test]
fn test_context_original_doc_unchanged() {
    let doc = json!({
        "counter": {
            "value": 10,
            "label": "Original"
        }
    });
    let doc_cell = DocCell::new(doc.clone());
    let ctx = StateContext::new(&doc_cell);

    let counter = ctx.state::<CounterState>("counter");
    counter.set_value(999);
    counter.set_label("Modified");

    // Original doc should be unchanged
    assert_eq!(doc["counter"]["value"], 10);
    assert_eq!(doc["counter"]["label"], "Original");

    // Changes are only reflected after apply_patch
    let tracked = ctx.take_patch();
    let new_doc = apply_patch(&doc, &tracked).unwrap();

    // Original still unchanged
    assert_eq!(doc["counter"]["value"], 10);

    // New doc has changes
    assert_eq!(new_doc["counter"]["value"], 999);
}

// ============================================================================
// from_value / to_value tests via Context
// ============================================================================

#[test]
fn test_state_from_value_via_context() {
    let doc = json!({
        "counter": {
            "value": 42,
            "label": "Test"
        }
    });

    // Extract typed struct from JSON
    let counter: CounterState = CounterState::from_value(&doc["counter"]).unwrap();
    assert_eq!(counter.value, 42);
    assert_eq!(counter.label, "Test");
}

#[test]
fn test_state_to_value_via_context() {
    let counter = CounterState {
        value: 100,
        label: "Created".to_string(),
    };

    let value = counter.to_value();
    assert_eq!(value["value"], 100);
    assert_eq!(value["label"], "Created");
}

// ============================================================================
// Write-through-read tests
// ============================================================================

#[test]
fn test_state_context_write_through_read_same_ref() {
    let doc = DocCell::new(json!({
        "counter": {"value": 10, "label": "Original"}
    }));
    let ctx = StateContext::new(&doc);

    let counter = ctx.state::<CounterState>("counter");
    assert_eq!(counter.value().unwrap(), 10);

    // Write then read from the same ref
    counter.set_value(42);
    assert_eq!(counter.value().unwrap(), 42, "same-ref read must see the write");
    assert_eq!(counter.label().unwrap(), "Original", "untouched field unchanged");
}

#[test]
fn test_state_context_write_through_read_cross_ref() {
    let doc = DocCell::new(json!({
        "counter": {"value": 0, "label": "Start"}
    }));
    let ctx = StateContext::new(&doc);

    // Write via first state ref
    let counter1 = ctx.state::<CounterState>("counter");
    counter1.set_value(100);
    counter1.set_label("Updated");

    // Read via second state ref — must see the writes
    let counter2 = ctx.state::<CounterState>("counter");
    assert_eq!(counter2.value().unwrap(), 100, "cross-ref read must see the write");
    assert_eq!(counter2.label().unwrap(), "Updated");
}

