//! Tests for the Accessor pattern and State container.

use carve_state::{apply_patch, AccessorOps, State};
use carve_state_derive::CarveViewModel;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::HashMap;

// ============================================================================
// Test View Models
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize, CarveViewModel, PartialEq)]
struct Counter {
    value: i32,
    label: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, CarveViewModel, PartialEq)]
struct UserProfile {
    name: String,
    age: u32,
    email: Option<String>,
    tags: Vec<String>,
    metadata: HashMap<String, String>,
}

// ============================================================================
// State Container Tests
// ============================================================================

#[test]
fn test_state_new() {
    let state = State::new();
    assert_eq!(state.raw(), &json!({}));
}

#[test]
fn test_state_from_value() {
    let value = json!({"counter": 42});
    let state = State::from_value(value.clone());
    assert_eq!(state.raw(), &value);
}

#[test]
fn test_state_from_model() {
    let counter = Counter {
        value: 10,
        label: "clicks".to_string(),
    };
    let state = State::from_model(&counter);
    assert_eq!(state.raw()["value"], 10);
    assert_eq!(state.raw()["label"], "clicks");
}

#[test]
fn test_state_path_access() {
    let mut state = State::new();

    // Set
    state
        .set(carve_state::Path::root().key("counter"), 42)
        .unwrap();
    assert_eq!(
        state.get(&carve_state::Path::root().key("counter")),
        Some(&json!(42))
    );

    // Delete
    state
        .delete(carve_state::Path::root().key("counter"))
        .unwrap();
    assert_eq!(state.get(&carve_state::Path::root().key("counter")), None);
}

#[test]
fn test_state_read_write() {
    let mut state = State::new();

    // Write using typed writer
    let mut w = state.write::<Counter>();
    w.value(42);
    w.label("test");
    state.apply(&w.build()).unwrap();

    // Read using typed reader
    let r = state.read::<Counter>();
    assert_eq!(r.value().unwrap(), 42);
    assert_eq!(r.label().unwrap(), "test");
}

#[test]
fn test_state_serialization() {
    let mut state = State::new();
    state
        .set(carve_state::Path::root().key("name"), "Alice")
        .unwrap();

    let json = state.to_json();
    let restored = State::from_json(&json).unwrap();

    assert_eq!(state.raw(), restored.raw());
}

// ============================================================================
// Accessor Tests
// ============================================================================

#[test]
fn test_accessor_basic() {
    let doc = json!({"value": 10, "label": "test"});

    let accessor = Counter::access(&doc);

    // Read via field proxy
    assert_eq!(accessor.value().get().unwrap(), 10);
    assert_eq!(accessor.label().get().unwrap(), "test");

    // Write
    accessor.set_value(20);
    accessor.set_label("updated");

    // Check operations accumulated
    assert!(accessor.has_changes());
    assert_eq!(accessor.len(), 2);

    // Build and apply patch
    let patch = accessor.build();
    let new_doc = apply_patch(&doc, &patch).unwrap();

    assert_eq!(new_doc["value"], 20);
    assert_eq!(new_doc["label"], "updated");
}

#[test]
fn test_accessor_with_state() {
    let mut state = State::from_value(json!({"value": 5, "label": "counter"}));

    // Use accessor pattern
    // Note: We need to build() first to release the borrow before apply()
    let patch = {
        let accessor = state.access::<Counter>();
        let current = accessor.value().get().unwrap();
        accessor.set_value(current + 10);
        accessor.build() // Consumes accessor, releasing the borrow
    };
    state.apply(&patch).unwrap();

    // Verify changes
    let reader = state.read::<Counter>();
    assert_eq!(reader.value().unwrap(), 15);
}

#[test]
fn test_accessor_numeric_operators() {
    let doc = json!({"value": 10, "label": "test"});

    let accessor = Counter::access(&doc);

    // Use numeric operators via field proxy
    let mut value_field = accessor.value();
    value_field += 5;

    let patch = accessor.build();
    let new_doc = apply_patch(&doc, &patch).unwrap();

    // Increment operation should work
    assert_eq!(new_doc["value"], 15);
}

#[test]
fn test_accessor_option_field() {
    let doc = json!({"name": "Alice", "age": 30, "tags": [], "metadata": {}});

    let accessor = UserProfile::access(&doc);

    // Option field - initially missing
    let email_field = accessor.email();
    assert!(email_field.is_none());

    // Set option field
    accessor.set_email("alice@example.com".to_string());

    let patch = accessor.build();
    let new_doc = apply_patch(&doc, &patch).unwrap();

    assert_eq!(new_doc["email"], "alice@example.com");
}

#[test]
fn test_accessor_vec_field() {
    let doc = json!({"name": "Bob", "age": 25, "tags": ["rust"], "metadata": {}});

    let accessor = UserProfile::access(&doc);

    // Read vec
    let tags = accessor.tags().get().unwrap();
    assert_eq!(tags, vec!["rust".to_string()]);

    // Push to vec
    accessor.tags_push("programming");

    let patch = accessor.build();
    let new_doc = apply_patch(&doc, &patch).unwrap();

    assert_eq!(new_doc["tags"], json!(["rust", "programming"]));
}

#[test]
fn test_accessor_map_field() {
    let doc = json!({
        "name": "Charlie",
        "age": 35,
        "tags": [],
        "metadata": {"key1": "value1"}
    });

    let accessor = UserProfile::access(&doc);

    // Read map
    let metadata = accessor.metadata();
    assert_eq!(metadata.get_key("key1").unwrap(), "value1");
    assert!(metadata.contains_key("key1"));

    // Insert into map
    accessor.metadata_insert("key2", "value2".to_string());

    let patch = accessor.build();
    let new_doc = apply_patch(&doc, &patch).unwrap();

    assert_eq!(new_doc["metadata"]["key2"], "value2");
}

#[test]
fn test_accessor_delete() {
    let doc = json!({"value": 100, "label": "to_delete"});

    let accessor = Counter::access(&doc);
    accessor.delete_label();

    let patch = accessor.build();
    let new_doc = apply_patch(&doc, &patch).unwrap();

    assert!(new_doc.get("label").is_none());
    assert_eq!(new_doc["value"], 100);
}

#[test]
fn test_accessor_multiple_operations() {
    let mut state = State::from_value(json!({
        "name": "Test User",
        "age": 20,
        "email": null,
        "tags": ["initial"],
        "metadata": {}
    }));

    let patch = {
        let accessor = state.access::<UserProfile>();

        // Multiple operations
        accessor.set_name("Updated User".to_string());
        accessor.set_age(25);
        accessor.set_email("test@example.com".to_string());
        accessor.tags_push("new_tag");
        accessor.metadata_insert("created", "2024".to_string());

        assert_eq!(accessor.len(), 5);
        accessor.build()
    };
    state.apply(&patch).unwrap();

    // Verify all changes
    let reader = state.read::<UserProfile>();
    assert_eq!(reader.name().unwrap(), "Updated User");
    assert_eq!(reader.age().unwrap(), 25);
    assert_eq!(reader.email().unwrap(), Some("test@example.com".to_string()));
}

#[test]
fn test_accessor_empty_commit() {
    let mut state = State::from_value(json!({"value": 42, "label": "unchanged"}));

    // Access without making changes
    let patch = {
        let accessor = state.access::<Counter>();
        assert!(!accessor.has_changes());
        assert!(accessor.is_empty());
        accessor.build()
    };
    state.apply(&patch).unwrap();

    // State should be unchanged
    let reader = state.read::<Counter>();
    assert_eq!(reader.value().unwrap(), 42);
    assert_eq!(reader.label().unwrap(), "unchanged");
}

// ============================================================================
// Field Proxy Tests
// ============================================================================

#[test]
fn test_scalar_field_get_or() {
    let doc = json!({"value": 42, "label": "test"});

    let accessor = Counter::access(&doc);

    // Existing field
    assert_eq!(accessor.value().get_or(0), 42);

    // Non-existing field would use default
    let empty_doc = json!({});
    let accessor2 = Counter::access(&empty_doc);
    assert_eq!(accessor2.value().get_or(99), 99);
}

#[test]
fn test_vec_field_operations() {
    let doc = json!({
        "name": "Test",
        "age": 30,
        "tags": ["a", "b", "c"],
        "metadata": {}
    });

    let accessor = UserProfile::access(&doc);
    let tags = accessor.tags();

    // Read operations
    assert_eq!(tags.len().unwrap(), 3);
    assert!(!tags.is_empty().unwrap());
    assert_eq!(tags.get_at(1).unwrap(), "b");

    // Write operations
    tags.push("d".to_string());
    tags.insert(0, "first".to_string());

    let patch = accessor.build();
    let new_doc = apply_patch(&doc, &patch).unwrap();

    // Note: The actual result depends on patch application order
    assert!(new_doc["tags"].as_array().unwrap().contains(&json!("d")));
}

#[test]
fn test_map_field_operations() {
    let doc = json!({
        "name": "Test",
        "age": 30,
        "tags": [],
        "metadata": {"existing": "value"}
    });

    let accessor = UserProfile::access(&doc);
    let metadata = accessor.metadata();

    // Read operations
    assert_eq!(metadata.len().unwrap(), 1);
    assert!(metadata.contains_key("existing"));
    assert!(!metadata.contains_key("nonexistent"));

    let keys = metadata.keys().unwrap();
    assert_eq!(keys, vec!["existing".to_string()]);

    // Write operations
    metadata.insert("new_key", "new_value".to_string());
    metadata.remove("existing");

    let patch = accessor.build();
    let new_doc = apply_patch(&doc, &patch).unwrap();

    assert_eq!(new_doc["metadata"]["new_key"], "new_value");
    assert!(new_doc["metadata"].get("existing").is_none());
}
