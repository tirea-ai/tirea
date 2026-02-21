//! Tests for #[tirea(flatten)] attribute.
//!
//! Flatten allows nested struct fields to be read/written at the parent level
//! rather than under a separate key in JSON.
#![allow(missing_docs)]

use serde::{Deserialize, Serialize};
use serde_json::json;
use std::sync::Mutex;
use tirea_state::{apply_patch, DocCell, PatchSink, Path, State as StateTrait};
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
// Basic flatten structures
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize, State, PartialEq)]
pub struct InnerState {
    pub value: i32,
    pub label: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, State)]
pub struct OuterState {
    pub name: String,

    // flatten: inner fields are at the root level in JSON
    #[tirea(flatten)]
    pub inner: InnerState,
}

// ============================================================================
// Reader tests
// ============================================================================

#[test]
fn test_flatten_read_basic() {
    // Flatten: inner fields are at the root level, not under "inner" key
    let doc = json!({
        "name": "outer_name",
        "value": 42,
        "label": "inner_label"
    });

    let patch = with_state_ref::<OuterState, _>(&doc, Path::root(), |state| {
        // Access outer field
        assert_eq!(state.name().unwrap(), "outer_name");

        // Access flattened inner fields through inner()
        // Even though they're at root level in JSON, we still access via inner()
        let inner = state.inner();
        assert_eq!(inner.value().unwrap(), 42);
        assert_eq!(inner.label().unwrap(), "inner_label");
    });

    assert!(patch.is_empty());
}

#[test]
fn test_flatten_read_missing_inner_fields() {
    let doc = json!({
        "name": "outer_name"
        // inner fields (value, label) are missing
    });

    let patch = with_state_ref::<OuterState, _>(&doc, Path::root(), |state| {
        assert_eq!(state.name().unwrap(), "outer_name");

        // Accessing missing flattened fields should error
        let inner = state.inner();
        assert!(inner.value().is_err());
        assert!(inner.label().is_err());
    });

    assert!(patch.is_empty());
}

// ============================================================================
// Writer tests
// ============================================================================

#[test]
fn test_flatten_write_basic() {
    let doc = json!({});

    let patch = with_state_ref::<OuterState, _>(&doc, Path::root(), |state| {
        state.set_name("parent").unwrap();
        state.inner().set_value(100).unwrap();
        state.inner().set_label("child").unwrap();
    });

    let result = apply_patch(&doc, &patch).unwrap();

    // Flattened fields should be at root level, not under "inner"
    assert_eq!(result["name"], "parent");
    assert_eq!(result["value"], 100);
    assert_eq!(result["label"], "child");
    assert!(result.get("inner").is_none(), "Should not have 'inner' key");
}

#[test]
fn test_flatten_write_partial() {
    let doc = json!({
        "name": "existing",
        "value": 1,
        "label": "old"
    });

    let patch = with_state_ref::<OuterState, _>(&doc, Path::root(), |state| {
        // Only update some fields
        state.inner().set_value(999).unwrap();
    });

    let result = apply_patch(&doc, &patch).unwrap();

    // Only value should change
    assert_eq!(result["name"], "existing");
    assert_eq!(result["value"], 999);
    assert_eq!(result["label"], "old");
}

#[test]
fn test_flatten_write_patch_paths() {
    let doc = json!({});

    let patch = with_state_ref::<OuterState, _>(&doc, Path::root(), |state| {
        state.inner().set_value(42).unwrap();
    });

    // The patch should have path $.value, not $.inner.value
    assert_eq!(patch.len(), 1);

    let result = apply_patch(&doc, &patch).unwrap();
    assert_eq!(result["value"], 42);
    assert!(result.get("inner").is_none());
}

// ============================================================================
// Delete tests
// ============================================================================

#[test]
fn test_flatten_delete() {
    let doc = json!({
        "name": "outer",
        "value": 100,
        "label": "to_delete"
    });

    let patch = with_state_ref::<OuterState, _>(&doc, Path::root(), |state| {
        state.inner().delete_label().unwrap();
    });

    let result = apply_patch(&doc, &patch).unwrap();

    assert_eq!(result["name"], "outer");
    assert_eq!(result["value"], 100);
    assert!(result.get("label").is_none());
}

// ============================================================================
// Increment tests
// ============================================================================

#[test]
fn test_flatten_increment() {
    let doc = json!({
        "name": "counter",
        "value": 10,
        "label": "test"
    });

    let patch = with_state_ref::<OuterState, _>(&doc, Path::root(), |state| {
        state.inner().increment_value(5).unwrap();
    });

    let result = apply_patch(&doc, &patch).unwrap();
    assert_eq!(result["value"], 15);
}

// ============================================================================
// Nested path with flatten tests
// ============================================================================

#[test]
fn test_flatten_at_nested_path() {
    let doc = json!({
        "data": {
            "name": "nested",
            "value": 50,
            "label": "nested_label"
        }
    });

    let patch = with_state_ref::<OuterState, _>(&doc, tirea_state::path!("data"), |state| {
        assert_eq!(state.name().unwrap(), "nested");
        assert_eq!(state.inner().value().unwrap(), 50);

        state.inner().set_value(100).unwrap();
    });

    let result = apply_patch(&doc, &patch).unwrap();
    assert_eq!(result["data"]["value"], 100);
    assert!(result["data"].get("inner").is_none());
}

// ============================================================================
// Double nested with flatten tests
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize, State, PartialEq)]
pub struct DeepInner {
    pub deep_value: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, State, PartialEq)]
pub struct MiddleState {
    pub middle_name: String,
    #[tirea(flatten)]
    pub deep: DeepInner,
}

#[derive(Debug, Clone, Serialize, Deserialize, State)]
pub struct TopState {
    pub top_name: String,
    #[tirea(nested)]
    pub middle: MiddleState,
}

#[test]
fn test_nested_with_inner_flatten() {
    // TopState has nested MiddleState, which has flattened DeepInner
    let doc = json!({
        "top_name": "top",
        "middle": {
            "middle_name": "mid",
            "deep_value": 42
        }
    });

    let patch = with_state_ref::<TopState, _>(&doc, Path::root(), |state| {
        assert_eq!(state.top_name().unwrap(), "top");

        let middle = state.middle();
        assert_eq!(middle.middle_name().unwrap(), "mid");

        // deep_value is flattened into middle
        assert_eq!(middle.deep().deep_value().unwrap(), 42);

        // Write to flattened field
        middle.deep().set_deep_value(100).unwrap();
    });

    let result = apply_patch(&doc, &patch).unwrap();
    assert_eq!(result["middle"]["deep_value"], 100);
    // No nested "deep" key
    assert!(result["middle"].get("deep").is_none());
}

// ============================================================================
// Flatten with other attributes tests
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize, State, PartialEq)]
pub struct RenamedInner {
    #[tirea(rename = "renamed_value")]
    pub value: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize, State)]
pub struct OuterWithRenamedFlat {
    pub name: String,
    #[tirea(flatten)]
    pub inner: RenamedInner,
}

#[test]
fn test_flatten_with_rename() {
    let doc = json!({
        "name": "test",
        "renamed_value": 42
    });

    let patch = with_state_ref::<OuterWithRenamedFlat, _>(&doc, Path::root(), |state| {
        assert_eq!(state.inner().value().unwrap(), 42);

        state.inner().set_value(100).unwrap();
    });

    let result = apply_patch(&doc, &patch).unwrap();
    assert_eq!(result["renamed_value"], 100);
    assert!(result.get("value").is_none());
}

#[derive(Debug, Clone, Serialize, Deserialize, State, PartialEq)]
pub struct InnerWithDefault {
    #[tirea(default = "0")]
    pub count: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize, State)]
pub struct OuterWithDefaultFlat {
    pub name: String,
    #[tirea(flatten)]
    pub inner: InnerWithDefault,
}

#[test]
fn test_flatten_with_default() {
    let doc = json!({
        "name": "test"
        // count is missing, should use default
    });

    let patch = with_state_ref::<OuterWithDefaultFlat, _>(&doc, Path::root(), |state| {
        assert_eq!(state.name().unwrap(), "test");
        // Should return default value
        assert_eq!(state.inner().count().unwrap(), 0);
    });

    assert!(patch.is_empty());
}

// ============================================================================
// Multiple reads and writes
// ============================================================================

#[test]
fn test_flatten_multiple_operations() {
    let doc = json!({
        "name": "start",
        "value": 0,
        "label": "initial"
    });

    let patch = with_state_ref::<OuterState, _>(&doc, Path::root(), |state| {
        // Multiple operations on both outer and flattened fields
        state.set_name("updated").unwrap();
        state.inner().set_value(10).unwrap();
        state.inner().increment_value(5).unwrap();
        state.inner().set_label("final").unwrap();
    });

    // Should have 4 operations
    assert_eq!(patch.len(), 4);

    let result = apply_patch(&doc, &patch).unwrap();
    assert_eq!(result["name"], "updated");
    assert_eq!(result["value"], 15);
    assert_eq!(result["label"], "final");
}

// ============================================================================
// from_value / to_value with flatten
// ============================================================================

// Note: #[tirea(flatten)] only affects StateRef read/write operations.
// For serde from_value/to_value, you would also need #[serde(flatten)]
// on the struct field. Here we test that from_value/to_value work with
// the nested structure (as serde expects).

#[test]
fn test_flatten_from_value_with_nested_json() {
    // from_value uses serde, which expects nested structure without #[serde(flatten)]
    let value = json!({
        "name": "outer",
        "inner": {
            "value": 42,
            "label": "inner_label"
        }
    });

    let outer: OuterState = OuterState::from_value(&value).unwrap();
    assert_eq!(outer.name, "outer");
    assert_eq!(outer.inner.value, 42);
    assert_eq!(outer.inner.label, "inner_label");
}

#[test]
fn test_flatten_to_value() {
    let outer = OuterState {
        name: "outer".to_string(),
        inner: InnerState {
            value: 100,
            label: "inner".to_string(),
        },
    };

    let value = outer.to_value().unwrap();

    // Without #[serde(flatten)], to_value produces nested structure
    assert!(value.is_object());
    assert_eq!(value["name"], "outer");
    assert_eq!(value["inner"]["value"], 100);
    assert_eq!(value["inner"]["label"], "inner");
}
