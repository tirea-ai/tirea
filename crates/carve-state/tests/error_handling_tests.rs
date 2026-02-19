//! Tests for error handling and error messages.
//!
//! These tests verify that meaningful error messages are produced
//! when operations fail.

use carve_state::{
    apply_patch, path, CarveError, DocCell, Op, Patch, PatchSink, Path, State as StateTrait, StateContext,
};
use carve_state_derive::State;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::BTreeMap;
use std::sync::Mutex;

/// Helper to create a state ref and collect patches for testing.
fn with_state_ref<T: StateTrait, F>(doc: &serde_json::Value, path: Path, f: F) -> carve_state::Patch
where
    F: FnOnce(T::Ref<'_>),
{
    let doc_cell = DocCell::new(doc.clone());
    let ops = Mutex::new(Vec::new());
    let sink = PatchSink::new(&ops);
    let state_ref = T::state_ref(&doc_cell, path, sink);
    f(state_ref);
    carve_state::Patch::with_ops(ops.into_inner().unwrap())
}

// ============================================================================
// Test state types
// ============================================================================

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, State)]
struct SimpleState {
    name: String,
    count: i64,
    active: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, State)]
struct TypedState {
    int_val: i32,
    float_val: f64,
    string_val: String,
    bool_val: bool,
    vec_val: Vec<i32>,
    map_val: BTreeMap<String, String>,
    opt_val: Option<i32>,
}

// ============================================================================
// Path not found errors
// ============================================================================

#[test]
fn test_error_path_not_found_simple() {
    let doc = json!({
        "other_field": 123
    });

    let _ = with_state_ref::<SimpleState, _>(&doc, Path::root(), |state| {
        let result = state.name();
        assert!(result.is_err());

        let err = result.unwrap_err();
        let err_str = err.to_string();
        assert!(
            err_str.contains("not found") || err_str.contains("missing"),
            "Error should indicate path not found: {}",
            err_str
        );
    });
}

#[test]
fn test_error_path_not_found_nested() {
    let doc = json!({
        "level1": {
            "level2": {}
        }
    });

    let patch = Patch::new().with_op(Op::set(
        path!("level1", "level2", "level3", "value"),
        json!(1),
    ));

    // This should work - creates intermediate paths
    let result = apply_patch(&doc, &patch);
    assert!(result.is_ok());
}

#[test]
fn test_error_read_from_empty_doc() {
    let doc = json!({});

    let _ = with_state_ref::<SimpleState, _>(&doc, Path::root(), |state| {
        let result = state.name();
        assert!(result.is_err());
    });
}

// ============================================================================
// Type mismatch errors
// ============================================================================

#[test]
fn test_error_type_mismatch_int_as_string() {
    let doc = json!({
        "int_val": "not an int",
        "float_val": 0.0,
        "string_val": "",
        "bool_val": false,
        "vec_val": [],
        "map_val": {},
        "opt_val": null
    });

    let _ = with_state_ref::<TypedState, _>(&doc, Path::root(), |state| {
        let result = state.int_val();
        assert!(result.is_err());

        let err = result.unwrap_err();
        let err_str = err.to_string();
        // Should indicate type error
        assert!(
            err_str.contains("invalid") || err_str.contains("expected") || err_str.contains("type"),
            "Error should indicate type mismatch: {}",
            err_str
        );
    });
}

#[test]
fn test_error_type_mismatch_string_as_int() {
    let doc = json!({
        "int_val": 0,
        "float_val": 0.0,
        "string_val": 12345, // should be string
        "bool_val": false,
        "vec_val": [],
        "map_val": {},
        "opt_val": null
    });

    let _ = with_state_ref::<TypedState, _>(&doc, Path::root(), |state| {
        let result = state.string_val();
        assert!(result.is_err());
    });
}

#[test]
fn test_error_type_mismatch_array_as_object() {
    let doc = json!({
        "int_val": 0,
        "float_val": 0.0,
        "string_val": "",
        "bool_val": false,
        "vec_val": {"not": "array"}, // should be array
        "map_val": {},
        "opt_val": null
    });

    let _ = with_state_ref::<TypedState, _>(&doc, Path::root(), |state| {
        let result = state.vec_val();
        assert!(result.is_err());
    });
}

#[test]
fn test_error_type_mismatch_object_as_array() {
    let doc = json!({
        "int_val": 0,
        "float_val": 0.0,
        "string_val": "",
        "bool_val": false,
        "vec_val": [],
        "map_val": [1, 2, 3], // should be object
        "opt_val": null
    });

    let _ = with_state_ref::<TypedState, _>(&doc, Path::root(), |state| {
        let result = state.map_val();
        assert!(result.is_err());
    });
}

// ============================================================================
// Apply patch errors
// ============================================================================

#[test]
fn test_error_increment_non_number() {
    let doc = json!({"value": "not a number"});

    let patch = Patch::new().with_op(Op::Increment {
        path: path!("value"),
        amount: carve_state::Number::Int(1),
    });

    let result = apply_patch(&doc, &patch);
    assert!(result.is_err());

    let err = result.unwrap_err();
    let err_str = err.to_string();
    assert!(
        err_str.contains("type") || err_str.contains("number") || err_str.contains("increment"),
        "Error should indicate cannot increment non-number: {}",
        err_str
    );
}

#[test]
fn test_error_append_to_non_array() {
    let doc = json!({"items": "not an array"});

    let patch = Patch::new().with_op(Op::Append {
        path: path!("items"),
        value: json!("new item"),
    });

    let result = apply_patch(&doc, &patch);
    assert!(result.is_err());

    let err = result.unwrap_err();
    let err_str = err.to_string();
    assert!(
        err_str.contains("type") || err_str.contains("array") || err_str.contains("append"),
        "Error should indicate cannot append to non-array: {}",
        err_str
    );
}

#[test]
fn test_error_merge_with_non_object() {
    let doc = json!({"target": [1, 2, 3]});

    let patch = Patch::new().with_op(Op::MergeObject {
        path: path!("target"),
        value: json!({"key": "value"}),
    });

    let result = apply_patch(&doc, &patch);
    assert!(result.is_err());

    let err = result.unwrap_err();
    let err_str = err.to_string();
    assert!(
        err_str.contains("type") || err_str.contains("object") || err_str.contains("merge"),
        "Error should indicate cannot merge with non-object: {}",
        err_str
    );
}

#[test]
fn test_error_insert_invalid_index() {
    let doc = json!({"items": [1, 2, 3]});

    let patch = Patch::new().with_op(Op::Insert {
        path: path!("items"),
        index: 100, // out of bounds
        value: json!("new"),
    });

    let result = apply_patch(&doc, &patch);
    assert!(result.is_err());

    let err = result.unwrap_err();
    let err_str = err.to_string();
    assert!(
        err_str.contains("index") || err_str.contains("bounds") || err_str.contains("100"),
        "Error should indicate index out of bounds: {}",
        err_str
    );
}

#[test]
fn test_error_remove_from_non_array() {
    let doc = json!({"items": {"not": "array"}});

    let patch = Patch::new().with_op(Op::Remove {
        path: path!("items"),
        value: json!("something"),
    });

    let result = apply_patch(&doc, &patch);
    assert!(result.is_err());
}

// ============================================================================
// from_value errors
// ============================================================================

#[test]
fn test_error_from_value_missing_field() {
    let value = json!({
        "name": "test"
        // count and active are missing
    });

    let result = SimpleState::from_value(&value);
    assert!(result.is_err());

    let err = result.unwrap_err();
    let err_str = err.to_string();
    assert!(
        err_str.contains("missing") || err_str.contains("field"),
        "Error should indicate missing field: {}",
        err_str
    );
}

#[test]
fn test_error_from_value_wrong_type() {
    let value = json!({
        "name": 123, // should be string
        "count": 10,
        "active": true
    });

    let result = SimpleState::from_value(&value);
    assert!(result.is_err());
}

#[test]
fn test_error_from_value_null() {
    let value = serde_json::Value::Null;

    let result = SimpleState::from_value(&value);
    assert!(result.is_err());
}

// ============================================================================
// CarveError variants
// ============================================================================

#[test]
fn test_carve_error_path_not_found() {
    let err = CarveError::path_not_found(path!("some", "path"));
    let err_str = err.to_string();
    assert!(err_str.contains("some") || err_str.contains("path"));
}

#[test]
fn test_carve_error_type_mismatch() {
    let err = CarveError::type_mismatch(path!("test"), "string", "integer");
    let err_str = err.to_string();
    assert!(err_str.contains("string") || err_str.contains("integer"));
}

#[test]
fn test_carve_error_index_out_of_bounds() {
    let err = CarveError::index_out_of_bounds(path!("arr"), 10, 5);
    let err_str = err.to_string();
    assert!(err_str.contains("10") || err_str.contains("5"));
}

// ============================================================================
// Context error handling
// ============================================================================

#[test]
fn test_context_error_propagation() {
    let doc = json!({
        "counter": {
            "name": "test"
            // count is missing
        }
    });
    let doc_cell = DocCell::new(doc.clone());
    let ctx = StateContext::new(&doc_cell);

    let counter = ctx.state::<SimpleState>("counter");

    // name exists
    assert!(counter.name().is_ok());

    // count is missing
    let result = counter.count();
    assert!(result.is_err());
}

// ============================================================================
// Error recovery patterns
// ============================================================================

#[test]
fn test_error_recovery_with_default() {
    let doc = json!({
        "counter": {}
    });

    let _ = with_state_ref::<SimpleState, _>(&doc, path!("counter"), |state| {
        // Use unwrap_or for error recovery
        let count = state.count().unwrap_or(0);
        assert_eq!(count, 0);

        let name = state.name().unwrap_or_else(|_| "default".to_string());
        assert_eq!(name, "default");
    });
}

#[test]
fn test_error_check_before_read() {
    let doc = json!({
        "data": {
            "name": "test"
        }
    });

    let doc_cell = DocCell::new(doc.clone());
    let ctx = StateContext::new(&doc_cell);
    let state = ctx.state::<SimpleState>("data");

    // Pattern: check if read succeeds before using
    if state.count().is_ok() {
        // Use the value
    } else {
        // Handle missing/invalid
    }
}

// ============================================================================
// Multiple errors
// ============================================================================

#[test]
fn test_multiple_read_errors() {
    let doc = json!({});

    let _ = with_state_ref::<SimpleState, _>(&doc, Path::root(), |state| {
        // All reads should fail
        let name_err = state.name().is_err();
        let count_err = state.count().is_err();
        let active_err = state.active().is_err();

        assert!(name_err);
        assert!(count_err);
        assert!(active_err);
    });
}

// ============================================================================
// Nested structure errors
// ============================================================================

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, State)]
struct InnerState {
    inner_value: i32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, State)]
struct OuterState {
    outer_name: String,
    #[carve(nested)]
    inner: InnerState,
}

#[test]
fn test_error_nested_missing_inner() {
    let doc = json!({
        "outer_name": "test"
        // inner is missing
    });

    let _ = with_state_ref::<OuterState, _>(&doc, Path::root(), |state| {
        // outer_name exists
        assert!(state.outer_name().is_ok());

        // inner.inner_value should error
        let inner = state.inner();
        assert!(inner.inner_value().is_err());
    });
}

#[test]
fn test_error_nested_wrong_type() {
    let doc = json!({
        "outer_name": "test",
        "inner": "not an object"
    });

    let _ = with_state_ref::<OuterState, _>(&doc, Path::root(), |state| {
        let inner = state.inner();
        let result = inner.inner_value();
        assert!(result.is_err());
    });
}
