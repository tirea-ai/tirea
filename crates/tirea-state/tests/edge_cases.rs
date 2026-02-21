//! Edge case tests for tirea-state.

use serde_json::json;
use tirea_state::{apply_patch, path, JsonWriter, Op, Patch, Path, TireaError};

// ============================================================================
// apply_patch edge cases
// ============================================================================

#[test]
fn test_apply_empty_patch() {
    let doc = json!({"x": 1, "y": 2});
    let patch = Patch::new();
    let result = apply_patch(&doc, &patch).unwrap();
    assert_eq!(result, doc);
}

#[test]
fn test_apply_multiple_ops() {
    let doc = json!({});
    let patch = Patch::new()
        .with_op(Op::set(path!("a"), json!(1)))
        .with_op(Op::set(path!("b"), json!(2)))
        .with_op(Op::set(path!("c"), json!(3)));
    let result = apply_patch(&doc, &patch).unwrap();
    assert_eq!(result["a"], 1);
    assert_eq!(result["b"], 2);
    assert_eq!(result["c"], 3);
}

#[test]
fn test_apply_ops_in_order() {
    let doc = json!({});
    let patch = Patch::new()
        .with_op(Op::set(path!("x"), json!(1)))
        .with_op(Op::set(path!("x"), json!(2)))
        .with_op(Op::set(path!("x"), json!(3)));
    let result = apply_patch(&doc, &patch).unwrap();
    // Last write wins
    assert_eq!(result["x"], 3);
}

#[test]
fn test_set_replaces_entire_value() {
    let doc = json!({"user": {"name": "Alice", "age": 30}});
    let patch = Patch::new().with_op(Op::set(path!("user"), json!({"name": "Bob"})));
    let result = apply_patch(&doc, &patch).unwrap();
    assert_eq!(result["user"], json!({"name": "Bob"}));
    // age should be gone
    assert!(result["user"].get("age").is_none());
}

#[test]
fn test_set_nested_creates_parents() {
    let doc = json!({});
    let patch = Patch::new().with_op(Op::set(path!("a", "b", "c", "d"), json!(42)));
    let result = apply_patch(&doc, &patch).unwrap();
    assert_eq!(result["a"]["b"]["c"]["d"], 42);
}

#[test]
fn test_delete_nested_path() {
    let doc = json!({"a": {"b": {"c": 1, "d": 2}}});
    let patch = Patch::new().with_op(Op::delete(path!("a", "b", "c")));
    let result = apply_patch(&doc, &patch).unwrap();
    assert!(result["a"]["b"].get("c").is_none());
    assert_eq!(result["a"]["b"]["d"], 2);
}

#[test]
fn test_delete_from_array() {
    let doc = json!({"arr": [1, 2, 3]});
    let patch = Patch::new().with_op(Op::delete(path!("arr", 1)));
    let result = apply_patch(&doc, &patch).unwrap();
    assert_eq!(result["arr"], json!([1, 3]));
}

#[test]
fn test_append_to_nested_array() {
    let doc = json!({"a": {"b": {"items": [1]}}});
    let patch = Patch::new().with_op(Op::append(path!("a", "b", "items"), json!(2)));
    let result = apply_patch(&doc, &patch).unwrap();
    assert_eq!(result["a"]["b"]["items"], json!([1, 2]));
}

#[test]
fn test_append_to_non_array_error() {
    let doc = json!({"x": 1});
    let patch = Patch::new().with_op(Op::append(path!("x"), json!(2)));
    let result = apply_patch(&doc, &patch);
    assert!(matches!(
        result,
        Err(TireaError::AppendRequiresArray { .. })
    ));
}

#[test]
fn test_merge_creates_object() {
    let doc = json!({});
    let patch = Patch::new().with_op(Op::merge_object(path!("user"), json!({"name": "Alice"})));
    let result = apply_patch(&doc, &patch).unwrap();
    assert_eq!(result["user"]["name"], "Alice");
}

#[test]
fn test_merge_preserves_existing() {
    let doc = json!({"user": {"name": "Alice", "age": 30}});
    let patch = Patch::new().with_op(Op::merge_object(path!("user"), json!({"email": "a@b.c"})));
    let result = apply_patch(&doc, &patch).unwrap();
    assert_eq!(result["user"]["name"], "Alice");
    assert_eq!(result["user"]["age"], 30);
    assert_eq!(result["user"]["email"], "a@b.c");
}

#[test]
fn test_merge_with_non_object_error() {
    let doc = json!({"x": 1});
    let patch = Patch::new().with_op(Op::merge_object(path!("x"), json!({"y": 2})));
    let result = apply_patch(&doc, &patch);
    assert!(matches!(
        result,
        Err(TireaError::MergeRequiresObject { .. })
    ));
}

#[test]
fn test_increment_float() {
    let doc = json!({"x": 1.5});
    let patch = Patch::new().with_op(Op::increment(path!("x"), 0.5f64));
    let result = apply_patch(&doc, &patch).unwrap();
    assert_eq!(result["x"], 2.0);
}

#[test]
fn test_increment_int_with_float() {
    let doc = json!({"x": 10});
    let patch = Patch::new().with_op(Op::increment(path!("x"), 0.5f64));
    let result = apply_patch(&doc, &patch).unwrap();
    assert_eq!(result["x"], 10.5);
}

#[test]
fn test_increment_non_number_error() {
    let doc = json!({"x": "string"});
    let patch = Patch::new().with_op(Op::increment(path!("x"), 1i64));
    let result = apply_patch(&doc, &patch);
    assert!(matches!(
        result,
        Err(TireaError::NumericOperationOnNonNumber { .. })
    ));
}

#[test]
fn test_decrement_to_negative() {
    let doc = json!({"x": 5});
    let patch = Patch::new().with_op(Op::decrement(path!("x"), 10i64));
    let result = apply_patch(&doc, &patch).unwrap();
    assert_eq!(result["x"], -5);
}

#[test]
fn test_insert_at_beginning() {
    let doc = json!({"arr": [1, 2, 3]});
    let patch = Patch::new().with_op(Op::insert(path!("arr"), 0, json!(0)));
    let result = apply_patch(&doc, &patch).unwrap();
    assert_eq!(result["arr"], json!([0, 1, 2, 3]));
}

#[test]
fn test_insert_at_end() {
    let doc = json!({"arr": [1, 2, 3]});
    let patch = Patch::new().with_op(Op::insert(path!("arr"), 3, json!(4)));
    let result = apply_patch(&doc, &patch).unwrap();
    assert_eq!(result["arr"], json!([1, 2, 3, 4]));
}

#[test]
fn test_insert_oob_error() {
    let doc = json!({"arr": [1, 2]});
    let patch = Patch::new().with_op(Op::insert(path!("arr"), 10, json!(99)));
    let result = apply_patch(&doc, &patch);
    assert!(matches!(result, Err(TireaError::IndexOutOfBounds { .. })));
}

#[test]
fn test_remove_nonexistent_value_noop() {
    let doc = json!({"arr": [1, 2, 3]});
    let patch = Patch::new().with_op(Op::remove(path!("arr"), json!(99)));
    let result = apply_patch(&doc, &patch).unwrap();
    assert_eq!(result["arr"], json!([1, 2, 3]));
}

#[test]
fn test_remove_from_non_array_error() {
    let doc = json!({"x": 1});
    let patch = Patch::new().with_op(Op::remove(path!("x"), json!(1)));
    let result = apply_patch(&doc, &patch);
    assert!(matches!(result, Err(TireaError::TypeMismatch { .. })));
}

// ============================================================================
// Error handling tests
// ============================================================================

#[test]
fn test_error_path_not_found() {
    let doc = json!({});
    let patch = Patch::new().with_op(Op::increment(path!("nonexistent"), 1i64));
    let result = apply_patch(&doc, &patch);
    match result {
        Err(TireaError::PathNotFound { path }) => {
            assert_eq!(path, path!("nonexistent"));
        }
        _ => panic!("expected PathNotFound error"),
    }
}

#[test]
fn test_error_index_out_of_bounds_details() {
    let doc = json!({"arr": [1, 2, 3]});
    let patch = Patch::new().with_op(Op::set(path!("arr", 5), json!(99)));
    let result = apply_patch(&doc, &patch);
    match result {
        Err(TireaError::IndexOutOfBounds { path, index, len }) => {
            // Path includes the full path including the array segment
            assert_eq!(path, path!("arr", 5));
            assert_eq!(index, 5);
            assert_eq!(len, 3);
        }
        _ => panic!("expected IndexOutOfBounds error"),
    }
}

#[test]
fn test_error_type_mismatch_details() {
    // Insert requires an array target, so using it on a string produces TypeMismatch
    let doc = json!({"x": "string"});
    let patch = Patch::new().with_op(Op::insert(path!("x"), 0, json!(1)));
    let result = apply_patch(&doc, &patch);
    match result {
        Err(TireaError::TypeMismatch {
            path,
            expected,
            found,
        }) => {
            assert_eq!(path, path!("x"));
            assert_eq!(expected, "array");
            assert_eq!(found, "string");
        }
        _ => panic!("expected TypeMismatch error, got {:?}", result),
    }
}

#[test]
fn test_error_display_format() {
    let err = TireaError::path_not_found(path!("foo", "bar"));
    let msg = format!("{}", err);
    assert!(msg.contains("path not found"));
}

// ============================================================================
// JsonWriter edge cases
// ============================================================================

#[test]
fn test_writer_empty_build() {
    let writer = JsonWriter::new();
    let patch = writer.build();
    assert!(patch.is_empty());
}

#[test]
fn test_writer_chained_ops() {
    let mut writer = JsonWriter::new();
    writer
        .set(path!("a"), json!(1))
        .set(path!("b"), json!(2))
        .delete(path!("c"));
    let patch = writer.build();
    assert_eq!(patch.len(), 3);
}

#[test]
fn test_writer_with_base_path() {
    let mut writer = JsonWriter::at(path!("users", "alice"));
    writer
        .set(path!("name"), json!("Alice"))
        .set(path!("age"), json!(30));
    let patch = writer.build();

    let doc = json!({});
    let result = apply_patch(&doc, &patch).unwrap();
    assert_eq!(result["users"]["alice"]["name"], "Alice");
    assert_eq!(result["users"]["alice"]["age"], 30);
}

#[test]
fn test_writer_merge_combines_ops() {
    let mut writer1 = JsonWriter::new();
    writer1.set(path!("a"), json!(1));

    let mut writer2 = JsonWriter::new();
    writer2.set(path!("b"), json!(2));

    writer1.merge(writer2);
    let patch = writer1.build();

    let doc = json!({});
    let result = apply_patch(&doc, &patch).unwrap();
    assert_eq!(result["a"], 1);
    assert_eq!(result["b"], 2);
}

#[test]
fn test_writer_all_op_types() {
    let doc = json!({
        "counter": 0,
        "items": [1, 2],
        "user": {}
    });

    let mut writer = JsonWriter::new();
    writer
        .set(path!("name"), json!("test"))
        .increment(path!("counter"), 5i64)
        .append(path!("items"), json!(3))
        .merge_object(path!("user"), json!({"name": "Bob"}));
    let patch = writer.build();

    let result = apply_patch(&doc, &patch).unwrap();
    assert_eq!(result["name"], "test");
    assert_eq!(result["counter"], 5);
    assert_eq!(result["items"], json!([1, 2, 3]));
    assert_eq!(result["user"]["name"], "Bob");
}

// ============================================================================
// Serde roundtrip tests
// ============================================================================

#[test]
fn test_patch_serde_roundtrip() {
    let patch = Patch::new()
        .with_op(Op::set(path!("a", "b"), json!(42)))
        .with_op(Op::delete(path!("c")))
        .with_op(Op::append(path!("items"), json!("item")));

    let json_str = serde_json::to_string(&patch).unwrap();
    let restored: Patch = serde_json::from_str(&json_str).unwrap();

    assert_eq!(patch.len(), restored.len());
}

#[test]
fn test_path_serde_roundtrip() {
    let path = path!("users", 0, "profile", "name");
    let json_str = serde_json::to_string(&path).unwrap();
    let restored: Path = serde_json::from_str(&json_str).unwrap();
    assert_eq!(path, restored);
}

#[test]
fn test_op_variants_serde() {
    let ops = vec![
        Op::set(path!("x"), json!(1)),
        Op::delete(path!("y")),
        Op::append(path!("arr"), json!(1)),
        Op::merge_object(path!("obj"), json!({"k": "v"})),
        Op::increment(path!("n"), 5i64),
        Op::decrement(path!("n"), 3f64),
        Op::insert(path!("arr"), 0, json!("first")),
        Op::remove(path!("arr"), json!("item")),
    ];

    for op in ops {
        let json_str = serde_json::to_string(&op).unwrap();
        let restored: Op = serde_json::from_str(&json_str).unwrap();
        assert_eq!(op, restored);
    }
}

// ============================================================================
// Path edge cases
// ============================================================================

#[test]
fn test_path_root() {
    let path = Path::root();
    assert!(path.is_empty());
    assert_eq!(path.len(), 0);
}

#[test]
fn test_path_join() {
    let base = path!("users");
    let sub = path!("alice", "profile");
    let joined = base.join(&sub);
    assert_eq!(joined, path!("users", "alice", "profile"));
}

#[test]
fn test_path_parent() {
    let path = path!("a", "b", "c");
    let parent = path.parent().unwrap();
    assert_eq!(parent, path!("a", "b"));

    let root = Path::root();
    assert!(root.parent().is_none());
}

#[test]
fn test_path_with_special_chars() {
    let path = path!("foo.bar", "baz/qux");
    let json_str = serde_json::to_string(&path).unwrap();
    let restored: Path = serde_json::from_str(&json_str).unwrap();
    assert_eq!(path, restored);
}
