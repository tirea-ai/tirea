//! Integration tests for CarveViewModel derive macro.

use carve_state::{apply_patch, path, CarveResult, CarveViewModel, Path};
use carve_state_derive::CarveViewModel;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::BTreeMap;

// ============================================================================
// Basic struct tests
// ============================================================================

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, CarveViewModel)]
struct SimpleStruct {
    name: String,
    age: u32,
    active: bool,
}

#[test]
fn test_simple_struct_reader() {
    let doc = json!({
        "name": "Alice",
        "age": 30,
        "active": true
    });

    let reader = SimpleStruct::read(&doc);

    assert_eq!(reader.name().unwrap(), "Alice");
    assert_eq!(reader.age().unwrap(), 30);
    assert_eq!(reader.active().unwrap(), true);
}

#[test]
fn test_simple_struct_reader_get() {
    let doc = json!({
        "name": "Bob",
        "age": 25,
        "active": false
    });

    let reader = SimpleStruct::read(&doc);
    let value = reader.get().unwrap();

    assert_eq!(value.name, "Bob");
    assert_eq!(value.age, 25);
    assert_eq!(value.active, false);
}

#[test]
fn test_simple_struct_reader_has() {
    let doc = json!({
        "name": "Charlie"
    });

    let reader = SimpleStruct::read(&doc);

    assert!(reader.has_name());
    assert!(!reader.has_age());
    assert!(!reader.has_active());
}

#[test]
fn test_simple_struct_writer() {
    let mut writer = SimpleStruct::write();
    writer.name("David");
    writer.age(40);
    writer.active(true);

    let patch = writer.build();
    assert_eq!(patch.len(), 3);

    let doc = json!({});
    let result = apply_patch(&doc, &patch).unwrap();

    assert_eq!(result["name"], "David");
    assert_eq!(result["age"], 40);
    assert_eq!(result["active"], true);
}

#[test]
fn test_simple_struct_writer_delete() {
    let mut writer = SimpleStruct::write();
    writer.delete_name();
    writer.delete_age();

    let patch = writer.build();
    assert_eq!(patch.len(), 2);

    let doc = json!({"name": "Eve", "age": 35, "active": true});
    let result = apply_patch(&doc, &patch).unwrap();

    assert!(result.get("name").is_none());
    assert!(result.get("age").is_none());
    assert_eq!(result["active"], true);
}

#[test]
fn test_simple_struct_from_value() {
    let value = json!({
        "name": "Frank",
        "age": 50,
        "active": false
    });

    let s = SimpleStruct::from_value(&value).unwrap();

    assert_eq!(s.name, "Frank");
    assert_eq!(s.age, 50);
    assert_eq!(s.active, false);
}

#[test]
fn test_simple_struct_to_value() {
    let s = SimpleStruct {
        name: "Grace".to_string(),
        age: 28,
        active: true,
    };

    let value = s.to_value();

    assert_eq!(value["name"], "Grace");
    assert_eq!(value["age"], 28);
    assert_eq!(value["active"], true);
}

// ============================================================================
// Struct with Option fields
// ============================================================================

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, CarveViewModel)]
struct WithOption {
    required: String,
    optional: Option<i32>,
}

#[test]
fn test_option_field_some() {
    let doc = json!({
        "required": "test",
        "optional": 42
    });

    let reader = WithOption::read(&doc);
    assert_eq!(reader.required().unwrap(), "test");
    assert_eq!(reader.optional().unwrap(), Some(42));
}

#[test]
fn test_option_field_none() {
    let doc = json!({
        "required": "test",
        "optional": null
    });

    let reader = WithOption::read(&doc);
    assert_eq!(reader.optional().unwrap(), None);
}

#[test]
fn test_option_field_missing() {
    let doc = json!({
        "required": "test"
    });

    let reader = WithOption::read(&doc);
    // Missing field should also return None for Option
    assert_eq!(reader.optional().unwrap(), None);
}

#[test]
fn test_option_writer_set_none() {
    let mut writer = WithOption::write();
    writer.required("test");
    writer.optional_none();

    let patch = writer.build();

    let doc = json!({"required": "old", "optional": 100});
    let result = apply_patch(&doc, &patch).unwrap();

    assert_eq!(result["required"], "test");
    assert!(result["optional"].is_null());
}

// ============================================================================
// Struct with Vec fields
// ============================================================================

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, CarveViewModel)]
struct WithVec {
    items: Vec<String>,
    numbers: Vec<i32>,
}

#[test]
fn test_vec_field_read() {
    let doc = json!({
        "items": ["a", "b", "c"],
        "numbers": [1, 2, 3]
    });

    let reader = WithVec::read(&doc);
    assert_eq!(reader.items().unwrap(), vec!["a", "b", "c"]);
    assert_eq!(reader.numbers().unwrap(), vec![1, 2, 3]);
}

#[test]
fn test_vec_field_write_set() {
    let mut writer = WithVec::write();
    writer.items(vec!["x".to_string(), "y".to_string()]);
    writer.numbers(vec![10, 20]);

    let patch = writer.build();

    let doc = json!({});
    let result = apply_patch(&doc, &patch).unwrap();

    assert_eq!(result["items"], json!(["x", "y"]));
    assert_eq!(result["numbers"], json!([10, 20]));
}

#[test]
fn test_vec_field_write_push() {
    let mut writer = WithVec::write();
    writer.items_push("first");
    writer.items_push("second");
    writer.numbers_push(1);
    writer.numbers_push(2);

    let patch = writer.build();

    let doc = json!({"items": [], "numbers": []});
    let result = apply_patch(&doc, &patch).unwrap();

    assert_eq!(result["items"], json!(["first", "second"]));
    assert_eq!(result["numbers"], json!([1, 2]));
}

// ============================================================================
// Struct with BTreeMap fields
// ============================================================================

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, CarveViewModel)]
struct WithMap {
    metadata: BTreeMap<String, String>,
    scores: BTreeMap<String, i32>,
}

#[test]
fn test_map_field_read() {
    let doc = json!({
        "metadata": {"key1": "value1", "key2": "value2"},
        "scores": {"alice": 100, "bob": 85}
    });

    let reader = WithMap::read(&doc);
    let metadata = reader.metadata().unwrap();
    assert_eq!(metadata.get("key1"), Some(&"value1".to_string()));
    assert_eq!(metadata.get("key2"), Some(&"value2".to_string()));

    let scores = reader.scores().unwrap();
    assert_eq!(scores.get("alice"), Some(&100));
    assert_eq!(scores.get("bob"), Some(&85));
}

#[test]
fn test_map_field_write_set() {
    let mut writer = WithMap::write();

    let mut metadata = BTreeMap::new();
    metadata.insert("k".to_string(), "v".to_string());
    writer.metadata(metadata);

    let patch = writer.build();

    let doc = json!({});
    let result = apply_patch(&doc, &patch).unwrap();

    assert_eq!(result["metadata"]["k"], "v");
}

#[test]
fn test_map_field_write_insert() {
    let mut writer = WithMap::write();
    writer.metadata_insert("key1", "value1");
    writer.metadata_insert("key2", "value2");
    writer.scores_insert("player1", 100);

    let patch = writer.build();

    let doc = json!({"metadata": {}, "scores": {}});
    let result = apply_patch(&doc, &patch).unwrap();

    assert_eq!(result["metadata"]["key1"], "value1");
    assert_eq!(result["metadata"]["key2"], "value2");
    assert_eq!(result["scores"]["player1"], 100);
}

// ============================================================================
// Nested struct tests
// ============================================================================

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, CarveViewModel)]
pub struct Inner {
    pub value: i32,
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, CarveViewModel)]
pub struct Outer {
    pub name: String,
    #[carve(nested)]
    pub inner: Inner,
}

#[test]
fn test_nested_struct_reader() {
    let doc = json!({
        "name": "outer",
        "inner": {
            "value": 42,
            "label": "nested"
        }
    });

    let reader = Outer::read(&doc);
    assert_eq!(reader.name().unwrap(), "outer");

    let inner_reader = reader.inner();
    assert_eq!(inner_reader.value().unwrap(), 42);
    assert_eq!(inner_reader.label().unwrap(), "nested");
}

#[test]
fn test_nested_struct_writer() {
    let mut writer = Outer::write();
    writer.name("parent");
    writer.inner().value(100);
    writer.inner().label("child");

    let patch = writer.build();

    let doc = json!({});
    let result = apply_patch(&doc, &patch).unwrap();

    assert_eq!(result["name"], "parent");
    assert_eq!(result["inner"]["value"], 100);
    assert_eq!(result["inner"]["label"], "child");
}

#[test]
fn test_nested_writer_auto_merge() {
    let mut writer = Outer::write();

    // Multiple nested operations should all be captured
    {
        let mut inner = writer.inner();
        inner.value(1);
        inner.label("first");
    }
    {
        let mut inner = writer.inner();
        inner.value(2);
        // Overwrites previous value
    }

    let patch = writer.build();
    // Should have: name (none), inner.value x2, inner.label x1
    assert!(patch.len() >= 3);
}

// ============================================================================
// Attribute tests: #[carve(rename)]
// ============================================================================

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, CarveViewModel)]
struct WithRename {
    #[carve(rename = "display_name")]
    name: String,
    #[carve(rename = "user_age")]
    age: u32,
}

#[test]
fn test_rename_attribute_read() {
    let doc = json!({
        "display_name": "Alice",
        "user_age": 30
    });

    let reader = WithRename::read(&doc);
    assert_eq!(reader.name().unwrap(), "Alice");
    assert_eq!(reader.age().unwrap(), 30);
}

#[test]
fn test_rename_attribute_write() {
    let mut writer = WithRename::write();
    writer.name("Bob");
    writer.age(25);

    let patch = writer.build();

    let doc = json!({});
    let result = apply_patch(&doc, &patch).unwrap();

    assert_eq!(result["display_name"], "Bob");
    assert_eq!(result["user_age"], 25);
    // Original names should not exist
    assert!(result.get("name").is_none());
    assert!(result.get("age").is_none());
}

// ============================================================================
// Attribute tests: #[carve(default)]
// ============================================================================

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, CarveViewModel)]
struct WithDefault {
    name: String,
    #[carve(default = "0")]
    count: i32,
    #[carve(default = "String::from(\"default\")")]
    label: String,
}

#[test]
fn test_default_attribute_missing_field() {
    let doc = json!({
        "name": "test"
    });

    let reader = WithDefault::read(&doc);
    assert_eq!(reader.name().unwrap(), "test");
    assert_eq!(reader.count().unwrap(), 0);
    assert_eq!(reader.label().unwrap(), "default");
}

#[test]
fn test_default_attribute_present_field() {
    let doc = json!({
        "name": "test",
        "count": 42,
        "label": "custom"
    });

    let reader = WithDefault::read(&doc);
    assert_eq!(reader.count().unwrap(), 42);
    assert_eq!(reader.label().unwrap(), "custom");
}

// ============================================================================
// Attribute tests: #[carve(skip)]
// Note: Skipped fields require Default trait since get() can't read them
// ============================================================================

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, CarveViewModel)]
struct WithSkip {
    visible: String,
    #[carve(skip)]
    #[serde(skip, default)]
    hidden: String,
}

impl Default for WithSkip {
    fn default() -> Self {
        Self {
            visible: String::new(),
            hidden: String::from("default_hidden"),
        }
    }
}

#[test]
fn test_skip_attribute() {
    let doc = json!({
        "visible": "can see this"
    });

    let reader = WithSkip::read(&doc);
    assert_eq!(reader.visible().unwrap(), "can see this");
    // hidden field should not have a reader method
    // (compile-time verification - no has_hidden or hidden() method)
}

#[test]
fn test_skip_attribute_writer() {
    let mut writer = WithSkip::write();
    writer.visible("test");
    // hidden field should not have a writer method
    // (compile-time verification - no hidden() setter)

    let patch = writer.build();
    assert_eq!(patch.len(), 1);
}

// ============================================================================
// Framework integration: using CarveViewModel trait generically
// ============================================================================

fn generic_read<T: CarveViewModel>(doc: &serde_json::Value) -> CarveResult<T> {
    T::from_value(doc)
}

fn generic_write_at_path<T: CarveViewModel>(base: Path) -> T::Writer {
    T::writer(base)
}

#[test]
fn test_generic_trait_usage() {
    let doc = json!({
        "name": "generic",
        "age": 99,
        "active": true
    });

    let s: SimpleStruct = generic_read(&doc).unwrap();
    assert_eq!(s.name, "generic");

    let writer = generic_write_at_path::<SimpleStruct>(path!("user"));
    assert!(writer.is_empty());
}

#[test]
fn test_reader_at_path() {
    let doc = json!({
        "users": {
            "alice": {
                "name": "Alice",
                "age": 30,
                "active": true
            }
        }
    });

    // Use the trait method with a custom path
    let reader = SimpleStruct::reader(&doc, path!("users", "alice"));
    assert_eq!(reader.name().unwrap(), "Alice");
}

#[test]
fn test_writer_at_path() {
    let mut writer = SimpleStruct::writer(path!("users", "bob"));
    writer.name("Bob");
    writer.age(25);

    let patch = writer.build();

    let doc = json!({"users": {}});
    let result = apply_patch(&doc, &patch).unwrap();

    assert_eq!(result["users"]["bob"]["name"], "Bob");
    assert_eq!(result["users"]["bob"]["age"], 25);
}

// ============================================================================
// Roundtrip tests
// ============================================================================

#[test]
fn test_full_roundtrip() {
    let original = SimpleStruct {
        name: "Roundtrip".to_string(),
        age: 42,
        active: true,
    };

    // Struct -> Value
    let value = original.to_value();

    // Value -> Struct
    let restored = SimpleStruct::from_value(&value).unwrap();

    assert_eq!(original, restored);
}

#[test]
fn test_read_write_roundtrip() {
    // Start with empty doc
    let mut doc = json!({});

    // Write using writer
    let mut writer = SimpleStruct::write();
    writer.name("Test");
    writer.age(100);
    writer.active(false);

    let patch = writer.build();
    doc = apply_patch(&doc, &patch).unwrap();

    // Read back using reader
    let reader = SimpleStruct::read(&doc);
    assert_eq!(reader.name().unwrap(), "Test");
    assert_eq!(reader.age().unwrap(), 100);
    assert_eq!(reader.active().unwrap(), false);

    // Get full struct
    let s = reader.get().unwrap();
    assert_eq!(s.name, "Test");
    assert_eq!(s.age, 100);
    assert_eq!(s.active, false);
}
