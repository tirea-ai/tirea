//! Integration tests for State derive macro.
#![allow(missing_docs)]

use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::BTreeMap;
use std::sync::Mutex;
use tirea_state::{
    apply_patch, path, DocCell, PatchSink, Path, State as StateTrait, StateExt, TireaError,
    TireaResult,
};
use tirea_state_derive::State;

/// Helper to create a state ref and collect patches for testing.
fn with_state_ref<T: StateTrait, F>(doc: &serde_json::Value, path: Path, f: F) -> tirea_state::Patch
where
    F: FnOnce(T::Ref<'_>),
{
    let doc_cell = DocCell::new(doc.clone());
    let ops = Mutex::new(Vec::new());
    let sink = PatchSink::new(&ops);
    let state_ref = T::state_ref(&doc_cell, path, sink);
    f(state_ref);
    tirea_state::Patch::with_ops(ops.into_inner().unwrap())
}

// ============================================================================
// Basic struct tests
// ============================================================================

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, State)]
struct SimpleStruct {
    name: String,
    age: u32,
    active: bool,
}

#[test]
fn test_simple_struct_read() {
    let doc = json!({
        "name": "Alice",
        "age": 30,
        "active": true
    });

    let patch = with_state_ref::<SimpleStruct, _>(&doc, Path::root(), |state| {
        assert_eq!(state.name().unwrap(), "Alice");
        assert_eq!(state.age().unwrap(), 30);
        assert!(state.active().unwrap());
    });

    assert!(patch.is_empty());
}

#[test]
fn test_simple_struct_write() {
    let doc = json!({});

    let patch = with_state_ref::<SimpleStruct, _>(&doc, Path::root(), |state| {
        state.set_name("David").unwrap();
        state.set_age(40).unwrap();
        state.set_active(true).unwrap();
    });

    assert_eq!(patch.len(), 3);

    let result = apply_patch(&doc, &patch).unwrap();
    assert_eq!(result["name"], "David");
    assert_eq!(result["age"], 40);
    assert_eq!(result["active"], true);
}

#[test]
fn test_simple_struct_delete() {
    let doc = json!({"name": "Eve", "age": 35, "active": true});

    let patch = with_state_ref::<SimpleStruct, _>(&doc, Path::root(), |state| {
        state.delete_name().unwrap();
        state.delete_age().unwrap();
    });

    assert_eq!(patch.len(), 2);

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
    assert!(!s.active);
}

#[test]
fn test_simple_struct_to_value() {
    let s = SimpleStruct {
        name: "Grace".to_string(),
        age: 28,
        active: true,
    };

    let value = s.to_value().unwrap();

    assert_eq!(value["name"], "Grace");
    assert_eq!(value["age"], 28);
    assert_eq!(value["active"], true);
}

// ============================================================================
// Struct with Option fields
// ============================================================================

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, State)]
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

    let patch = with_state_ref::<WithOption, _>(&doc, Path::root(), |state| {
        assert_eq!(state.required().unwrap(), "test");
        assert_eq!(state.optional().unwrap(), Some(42));
    });

    assert!(patch.is_empty());
}

#[test]
fn test_option_field_none() {
    let doc = json!({
        "required": "test",
        "optional": null
    });

    let patch = with_state_ref::<WithOption, _>(&doc, Path::root(), |state| {
        assert_eq!(state.optional().unwrap(), None);
    });

    assert!(patch.is_empty());
}

#[test]
fn test_option_field_missing() {
    let doc = json!({
        "required": "test"
    });

    let patch = with_state_ref::<WithOption, _>(&doc, Path::root(), |state| {
        // Missing field should also return None for Option
        assert_eq!(state.optional().unwrap(), None);
    });

    assert!(patch.is_empty());
}

#[test]
fn test_option_writer_set_none() {
    let doc = json!({"required": "old", "optional": 100});

    let patch = with_state_ref::<WithOption, _>(&doc, Path::root(), |state| {
        state.set_required("test").unwrap();
        state.optional_none().unwrap();
    });

    let result = apply_patch(&doc, &patch).unwrap();
    assert_eq!(result["required"], "test");
    assert!(result["optional"].is_null());
}

// ============================================================================
// Struct with Vec fields
// ============================================================================

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, State)]
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

    let patch = with_state_ref::<WithVec, _>(&doc, Path::root(), |state| {
        assert_eq!(state.items().unwrap(), vec!["a", "b", "c"]);
        assert_eq!(state.numbers().unwrap(), vec![1, 2, 3]);
    });

    assert!(patch.is_empty());
}

#[test]
fn test_vec_field_write_set() {
    let doc = json!({});

    let patch = with_state_ref::<WithVec, _>(&doc, Path::root(), |state| {
        state
            .set_items(vec!["x".to_string(), "y".to_string()])
            .unwrap();
        state.set_numbers(vec![10, 20]).unwrap();
    });

    let result = apply_patch(&doc, &patch).unwrap();
    assert_eq!(result["items"], json!(["x", "y"]));
    assert_eq!(result["numbers"], json!([10, 20]));
}

#[test]
fn test_vec_field_write_push() {
    let doc = json!({"items": [], "numbers": []});

    let patch = with_state_ref::<WithVec, _>(&doc, Path::root(), |state| {
        state.items_push("first").unwrap();
        state.items_push("second").unwrap();
        state.numbers_push(1).unwrap();
        state.numbers_push(2).unwrap();
    });

    let result = apply_patch(&doc, &patch).unwrap();
    assert_eq!(result["items"], json!(["first", "second"]));
    assert_eq!(result["numbers"], json!([1, 2]));
}

// ============================================================================
// Struct with BTreeMap fields
// ============================================================================

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, State)]
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

    let patch = with_state_ref::<WithMap, _>(&doc, Path::root(), |state| {
        let metadata = state.metadata().unwrap();
        assert_eq!(metadata.get("key1"), Some(&"value1".to_string()));
        assert_eq!(metadata.get("key2"), Some(&"value2".to_string()));

        let scores = state.scores().unwrap();
        assert_eq!(scores.get("alice"), Some(&100));
        assert_eq!(scores.get("bob"), Some(&85));
    });

    assert!(patch.is_empty());
}

#[test]
fn test_map_field_write_set() {
    let doc = json!({});

    let patch = with_state_ref::<WithMap, _>(&doc, Path::root(), |state| {
        let mut metadata = BTreeMap::new();
        metadata.insert("k".to_string(), "v".to_string());
        state.set_metadata(metadata).unwrap();
    });

    let result = apply_patch(&doc, &patch).unwrap();
    assert_eq!(result["metadata"]["k"], "v");
}

#[test]
fn test_map_field_write_insert() {
    let doc = json!({"metadata": {}, "scores": {}});

    let patch = with_state_ref::<WithMap, _>(&doc, Path::root(), |state| {
        state.metadata_insert("key1", "value1").unwrap();
        state.metadata_insert("key2", "value2").unwrap();
        state.scores_insert("player1", 100).unwrap();
    });

    let result = apply_patch(&doc, &patch).unwrap();
    assert_eq!(result["metadata"]["key1"], "value1");
    assert_eq!(result["metadata"]["key2"], "value2");
    assert_eq!(result["scores"]["player1"], 100);
}

// ============================================================================
// Nested struct tests
// ============================================================================

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, State)]
pub struct Inner {
    pub value: i32,
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, State)]
pub struct Outer {
    pub name: String,
    #[tirea(nested)]
    pub inner: Inner,
}

#[test]
fn test_nested_struct_read() {
    let doc = json!({
        "name": "outer",
        "inner": {
            "value": 42,
            "label": "nested"
        }
    });

    let patch = with_state_ref::<Outer, _>(&doc, Path::root(), |state| {
        assert_eq!(state.name().unwrap(), "outer");

        let inner_ref = state.inner();
        assert_eq!(inner_ref.value().unwrap(), 42);
        assert_eq!(inner_ref.label().unwrap(), "nested");
    });

    assert!(patch.is_empty());
}

#[test]
fn test_nested_struct_write() {
    let doc = json!({});

    let patch = with_state_ref::<Outer, _>(&doc, Path::root(), |state| {
        state.set_name("parent").unwrap();
        state.inner().set_value(100).unwrap();
        state.inner().set_label("child").unwrap();
    });

    let result = apply_patch(&doc, &patch).unwrap();
    assert_eq!(result["name"], "parent");
    assert_eq!(result["inner"]["value"], 100);
    assert_eq!(result["inner"]["label"], "child");
}

// ============================================================================
// Attribute tests: #[tirea(rename)]
// ============================================================================

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, State)]
struct WithRename {
    #[tirea(rename = "display_name")]
    name: String,
    #[tirea(rename = "user_age")]
    age: u32,
}

#[test]
fn test_rename_attribute_read() {
    let doc = json!({
        "display_name": "Alice",
        "user_age": 30
    });

    let patch = with_state_ref::<WithRename, _>(&doc, Path::root(), |state| {
        assert_eq!(state.name().unwrap(), "Alice");
        assert_eq!(state.age().unwrap(), 30);
    });

    assert!(patch.is_empty());
}

#[test]
fn test_rename_attribute_write() {
    let doc = json!({});

    let patch = with_state_ref::<WithRename, _>(&doc, Path::root(), |state| {
        state.set_name("Bob").unwrap();
        state.set_age(25).unwrap();
    });

    let result = apply_patch(&doc, &patch).unwrap();
    assert_eq!(result["display_name"], "Bob");
    assert_eq!(result["user_age"], 25);
    // Original names should not exist
    assert!(result.get("name").is_none());
    assert!(result.get("age").is_none());
}

// ============================================================================
// Attribute tests: #[tirea(default)]
// ============================================================================

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, State)]
struct WithDefault {
    name: String,
    #[tirea(default = "0")]
    count: i32,
    #[tirea(default = "String::from(\"default\")")]
    label: String,
}

#[test]
fn test_default_attribute_missing_field() {
    let doc = json!({
        "name": "test"
    });

    let patch = with_state_ref::<WithDefault, _>(&doc, Path::root(), |state| {
        assert_eq!(state.name().unwrap(), "test");
        assert_eq!(state.count().unwrap(), 0);
        assert_eq!(state.label().unwrap(), "default");
    });

    assert!(patch.is_empty());
}

#[test]
fn test_default_attribute_present_field() {
    let doc = json!({
        "name": "test",
        "count": 42,
        "label": "custom"
    });

    let patch = with_state_ref::<WithDefault, _>(&doc, Path::root(), |state| {
        assert_eq!(state.count().unwrap(), 42);
        assert_eq!(state.label().unwrap(), "custom");
    });

    assert!(patch.is_empty());
}

// ============================================================================
// Attribute tests: #[tirea(skip)]
// ============================================================================

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, State)]
struct WithSkip {
    visible: String,
    #[tirea(skip)]
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

    let patch = with_state_ref::<WithSkip, _>(&doc, Path::root(), |state| {
        assert_eq!(state.visible().unwrap(), "can see this");
        // hidden field should not have a method
        // (compile-time verification - no hidden() method)
    });

    assert!(patch.is_empty());
}

#[test]
fn test_skip_attribute_write() {
    let doc = json!({});

    let patch = with_state_ref::<WithSkip, _>(&doc, Path::root(), |state| {
        state.set_visible("test").unwrap();
        // hidden field should not have a setter
        // (compile-time verification - no set_hidden() method)
    });

    assert_eq!(patch.len(), 1);
}

// ============================================================================
// Framework integration: using State trait generically
// ============================================================================

fn generic_from_value<T: StateTrait>(doc: &serde_json::Value) -> TireaResult<T> {
    T::from_value(doc)
}

#[test]
fn test_generic_trait_usage() {
    let doc = json!({
        "name": "generic",
        "age": 99,
        "active": true
    });

    let s: SimpleStruct = generic_from_value(&doc).unwrap();
    assert_eq!(s.name, "generic");
}

// ============================================================================
// State at path tests
// ============================================================================

#[test]
fn test_state_at_path() {
    let doc = json!({
        "users": {
            "alice": {
                "name": "Alice",
                "age": 30,
                "active": true
            }
        }
    });

    let patch = with_state_ref::<SimpleStruct, _>(&doc, path!("users", "alice"), |state| {
        assert_eq!(state.name().unwrap(), "Alice");
        assert_eq!(state.age().unwrap(), 30);
    });

    assert!(patch.is_empty());
}

#[test]
fn test_write_at_path() {
    let doc = json!({"users": {}});

    let patch = with_state_ref::<SimpleStruct, _>(&doc, path!("users", "bob"), |state| {
        state.set_name("Bob").unwrap();
        state.set_age(25).unwrap();
    });

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
    let value = original.to_value().unwrap();

    // Value -> Struct
    let restored = SimpleStruct::from_value(&value).unwrap();

    assert_eq!(original, restored);
}

#[test]
fn test_read_write_roundtrip() {
    // Start with empty doc
    let mut doc = json!({});

    // Write using state ref
    let patch = with_state_ref::<SimpleStruct, _>(&doc, Path::root(), |state| {
        state.set_name("Test").unwrap();
        state.set_age(100).unwrap();
        state.set_active(false).unwrap();
    });

    doc = apply_patch(&doc, &patch).unwrap();

    // Read back using state ref
    let patch = with_state_ref::<SimpleStruct, _>(&doc, Path::root(), |state| {
        assert_eq!(state.name().unwrap(), "Test");
        assert_eq!(state.age().unwrap(), 100);
        assert!(!state.active().unwrap());
    });

    assert!(patch.is_empty());
}

// ============================================================================
// Numeric increment/decrement tests
// ============================================================================

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, State)]
struct WithNumbers {
    count: i64,
    score: f64,
}

#[test]
fn test_increment_i64() {
    let doc = json!({"count": 10, "score": 1.5});

    let patch = with_state_ref::<WithNumbers, _>(&doc, Path::root(), |state| {
        state.increment_count(5).unwrap();
    });

    let result = apply_patch(&doc, &patch).unwrap();
    assert_eq!(result["count"], 15);
}

#[test]
fn test_decrement_i64() {
    let doc = json!({"count": 10, "score": 1.5});

    let patch = with_state_ref::<WithNumbers, _>(&doc, Path::root(), |state| {
        state.decrement_count(3).unwrap();
    });

    let result = apply_patch(&doc, &patch).unwrap();
    assert_eq!(result["count"], 7);
}

#[test]
fn test_increment_f64() {
    let doc = json!({"count": 10, "score": 1.5});

    let patch = with_state_ref::<WithNumbers, _>(&doc, Path::root(), |state| {
        state.increment_score(0.5).unwrap();
    });

    let result = apply_patch(&doc, &patch).unwrap();
    assert_eq!(result["score"], 2.0);
}

// ============================================================================
// State::to_patch() tests
// ============================================================================

#[test]
fn test_state_to_patch_basic() {
    let state = SimpleStruct {
        name: "Alice".to_string(),
        age: 30,
        active: true,
    };

    let patch = state.to_patch().unwrap();

    // Apply patch to empty document
    let doc = json!({});
    let result = apply_patch(&doc, &patch).unwrap();

    assert_eq!(result["name"], "Alice");
    assert_eq!(result["age"], 30);
    assert_eq!(result["active"], true);
}

#[test]
fn test_state_to_patch_overwrites_existing() {
    let state = SimpleStruct {
        name: "Bob".to_string(),
        age: 25,
        active: false,
    };

    let patch = state.to_patch().unwrap();

    // Apply patch to document with existing data
    let doc = json!({
        "name": "Alice",
        "age": 30,
        "active": true,
        "extra": "field"
    });
    let result = apply_patch(&doc, &patch).unwrap();

    // State values should overwrite at root
    assert_eq!(result["name"], "Bob");
    assert_eq!(result["age"], 25);
    assert_eq!(result["active"], false);
}

#[test]
fn test_state_to_patch_with_nested() {
    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize, State)]
    struct Inner {
        value: i32,
    }

    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize, State)]
    struct Outer {
        name: String,
        #[tirea(nested)]
        inner: Inner,
    }

    let state = Outer {
        name: "outer".to_string(),
        inner: Inner { value: 42 },
    };

    let patch = state.to_patch().unwrap();
    let doc = json!({});
    let result = apply_patch(&doc, &patch).unwrap();

    assert_eq!(result["name"], "outer");
    assert_eq!(result["inner"]["value"], 42);
}

// ============================================================================
// StateExt::at_root() tests
// ============================================================================

#[test]
fn test_state_ext_at_root_read() {
    let doc = DocCell::new(json!({
        "name": "Alice",
        "age": 30,
        "active": true
    }));

    let ops = Mutex::new(Vec::new());
    let sink = PatchSink::new(&ops);

    // Use StateExt::at_root() instead of State::state_ref()
    let state = SimpleStruct::at_root(&doc, sink);

    assert_eq!(state.name().unwrap(), "Alice");
    assert_eq!(state.age().unwrap(), 30);
    assert!(state.active().unwrap());
}

#[test]
fn test_state_ext_at_root_write() {
    let doc_value = json!({
        "name": "Alice",
        "age": 30,
        "active": true
    });
    let doc = DocCell::new(doc_value.clone());

    let ops = Mutex::new(Vec::new());
    let sink = PatchSink::new(&ops);

    let state = SimpleStruct::at_root(&doc, sink);
    state.set_name("Bob").unwrap();
    state.set_age(25).unwrap();

    drop(state);
    let patch = tirea_state::Patch::with_ops(ops.into_inner().unwrap());
    let result = apply_patch(&doc_value, &patch).unwrap();

    assert_eq!(result["name"], "Bob");
    assert_eq!(result["age"], 25);
    assert_eq!(result["active"], true); // unchanged
}

#[test]
fn test_state_ext_at_root_equivalent_to_state_ref_root() {
    let doc_value = json!({
        "name": "Test",
        "age": 100,
        "active": false
    });
    let doc = DocCell::new(doc_value.clone());

    // Using StateExt::at_root
    let ops1 = Mutex::new(Vec::new());
    let sink1 = PatchSink::new(&ops1);
    let state1 = SimpleStruct::at_root(&doc, sink1);
    let name1 = state1.name().unwrap();
    state1.set_age(200).unwrap();
    drop(state1);

    // Using State::state_ref with Path::root()
    let ops2 = Mutex::new(Vec::new());
    let sink2 = PatchSink::new(&ops2);
    let state2 = SimpleStruct::state_ref(&doc, Path::root(), sink2);
    let name2 = state2.name().unwrap();
    state2.set_age(200).unwrap();
    drop(state2);

    // Results should be identical
    assert_eq!(name1, name2);

    let patch1 = tirea_state::Patch::with_ops(ops1.into_inner().unwrap());
    let patch2 = tirea_state::Patch::with_ops(ops2.into_inner().unwrap());

    let result1 = apply_patch(&doc_value, &patch1).unwrap();
    let result2 = apply_patch(&doc_value, &patch2).unwrap();

    assert_eq!(result1, result2);
}

// ============================================================================
// Serialization failure behavior
// ============================================================================

#[derive(Debug, Clone, PartialEq)]
struct FailingSerialize;

impl Serialize for FailingSerialize {
    fn serialize<S>(&self, _serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        Err(serde::ser::Error::custom("intentional serialize failure"))
    }
}

impl<'de> Deserialize<'de> for FailingSerialize {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let _ = serde_json::Value::deserialize(deserializer)?;
        Ok(Self)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, State)]
struct WithFailingSerialize {
    bad: FailingSerialize,
}

#[test]
fn test_setter_serialization_failure_returns_error() {
    let doc = json!({});
    let _ = with_state_ref::<WithFailingSerialize, _>(&doc, Path::root(), |state| {
        let err = state.set_bad(FailingSerialize).unwrap_err();
        assert!(matches!(err, TireaError::Serialization(_)));
    });
}

#[test]
fn test_to_value_serialization_failure_returns_error() {
    let state = WithFailingSerialize {
        bad: FailingSerialize,
    };
    let err = state.to_value().unwrap_err();
    assert!(matches!(err, TireaError::Serialization(_)));
}
