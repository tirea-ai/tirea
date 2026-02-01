//! Tests for #[carve(flatten)] functionality.
//!
//! Flatten allows nested struct fields to be serialized/deserialized at the parent level
//! rather than under a separate key.

use carve_state::{apply_patch, AccessorOps, CarveViewModel};
use carve_state_derive::CarveViewModel as DeriveCarveViewModel;
use serde::{Deserialize, Serialize};
use serde_json::json;

// ============================================================================
// Basic flatten structures
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize, DeriveCarveViewModel, PartialEq)]
pub struct Inner {
    pub value: i32,
    pub label: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, DeriveCarveViewModel)]
pub struct Outer {
    pub name: String,

    // flatten is implicitly nested, no need to mark both
    #[carve(flatten)]
    pub inner: Inner,
}

// ============================================================================
// Reader tests
// ============================================================================

#[test]
fn test_flatten_reader() {
    // Flatten: inner fields are at the root level, not under "inner" key
    let doc = json!({
        "name": "x",
        "value": 1,
        "label": "y"
    });

    let reader = Outer::read(&doc);

    // Access outer field
    assert_eq!(reader.name().unwrap(), "x");

    // Access flattened inner fields through inner()
    // Even though they're at root level in JSON, we still access via inner()
    assert_eq!(reader.inner().value().unwrap(), 1);
    assert_eq!(reader.inner().label().unwrap(), "y");
}

#[test]
fn test_flatten_reader_missing_inner_fields() {
    let doc = json!({
        "name": "x"
        // inner fields missing
    });

    let reader = Outer::read(&doc);
    assert_eq!(reader.name().unwrap(), "x");

    // Accessing missing flattened fields should error
    assert!(reader.inner().value().is_err());
    assert!(reader.inner().label().is_err());
}

// ============================================================================
// Writer tests
// ============================================================================

#[test]
fn test_flatten_writer() {
    let mut writer = Outer::write();

    writer.name("x");
    {
        let mut inner_writer = writer.inner();
        inner_writer.value(10);
        inner_writer.label("L");
    }

    let patch = writer.build();
    let result = apply_patch(&json!({}), &patch).unwrap();

    // Flattened fields should be at root level, not under "inner"
    assert_eq!(result["name"], "x");
    assert_eq!(result["value"], 10);
    assert_eq!(result["label"], "L");
    assert!(result.get("inner").is_none(), "Should not have 'inner' key");
}

#[test]
fn test_flatten_writer_patch_paths() {
    let mut writer = Outer::write();

    writer.inner().value(42);

    let patch = writer.build();

    // Verify patch path is at root level
    assert_eq!(patch.len(), 1);
    let ops = patch.ops();
    match &ops[0] {
        carve_state::Op::Set { path, value } => {
            assert_eq!(path.to_string(), "$.value", "Flatten path should be $.value, not $.inner.value");
            assert_eq!(value, &json!(42));
        }
        other => panic!("Expected Set op, got: {:?}", other),
    }
}

// ============================================================================
// Accessor tests
// ============================================================================

#[test]
fn test_flatten_accessor() {
    let doc = json!({
        "name": "test",
        "value": 5,
        "label": "original"
    });

    let accessor = Outer::access(&doc);

    // Read flattened fields
    {
        let inner = accessor.inner();
        assert_eq!(inner.value().get().unwrap(), 5);
        assert_eq!(inner.label().get().unwrap(), "original");
    }

    // Modify flattened fields
    {
        let inner = accessor.inner();
        inner.set_value(99);
        inner.set_label("updated".to_string());
    }

    let patch = accessor.build();
    let result = apply_patch(&doc, &patch).unwrap();

    assert_eq!(result["name"], "test");
    assert_eq!(result["value"], 99);
    assert_eq!(result["label"], "updated");
}

// ============================================================================
// from_value tests
// ============================================================================

#[test]
fn test_flatten_from_value() {
    let value = json!({
        "name": "outer_name",
        "value": 100,
        "label": "inner_label"
    });

    let outer = Outer::from_value(&value).unwrap();

    assert_eq!(outer.name, "outer_name");
    assert_eq!(outer.inner.value, 100);
    assert_eq!(outer.inner.label, "inner_label");
}

#[test]
fn test_flatten_from_value_missing_inner_fields() {
    let value = json!({
        "name": "outer_name"
        // inner fields missing
    });

    let result = Outer::from_value(&value);
    assert!(result.is_err(), "Should error when flattened fields are missing");
}

// ============================================================================
// to_value tests
// ============================================================================

#[test]
fn test_flatten_to_value() {
    let outer = Outer {
        name: "test".to_string(),
        inner: Inner {
            value: 42,
            label: "label42".to_string(),
        },
    };

    let value = outer.to_value();

    // Flattened fields should be at root level
    assert_eq!(value["name"], "test");
    assert_eq!(value["value"], 42);
    assert_eq!(value["label"], "label42");
    assert!(value.get("inner").is_none(), "Should not have 'inner' key");
}

// ============================================================================
// Round-trip tests
// ============================================================================

#[test]
fn test_flatten_round_trip() {
    let original = Outer {
        name: "round_trip".to_string(),
        inner: Inner {
            value: 123,
            label: "test_label".to_string(),
        },
    };

    // to_value
    let value = original.to_value();

    // from_value
    let deserialized = Outer::from_value(&value).unwrap();

    assert_eq!(deserialized.name, original.name);
    assert_eq!(deserialized.inner.value, original.inner.value);
    assert_eq!(deserialized.inner.label, original.inner.label);
}

// ============================================================================
// Multiple flattened fields (potential key conflicts)
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize, DeriveCarveViewModel, PartialEq)]
pub struct Part1 {
    pub a: i32,
    pub b: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, DeriveCarveViewModel, PartialEq)]
pub struct Part2 {
    pub c: i32,
    pub d: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, DeriveCarveViewModel)]
pub struct Composite {
    pub name: String,

    #[carve(flatten)]
    pub part1: Part1,

    #[carve(flatten)]
    pub part2: Part2,
}

#[test]
fn test_multiple_flatten_fields() {
    let value = json!({
        "name": "composite",
        "a": 1,
        "b": "bee",
        "c": 3,
        "d": "dee"
    });

    let composite = Composite::from_value(&value).unwrap();

    assert_eq!(composite.name, "composite");
    assert_eq!(composite.part1.a, 1);
    assert_eq!(composite.part1.b, "bee");
    assert_eq!(composite.part2.c, 3);
    assert_eq!(composite.part2.d, "dee");

    // Round trip
    let serialized = composite.to_value();
    assert_eq!(serialized["name"], "composite");
    assert_eq!(serialized["a"], 1);
    assert_eq!(serialized["b"], "bee");
    assert_eq!(serialized["c"], 3);
    assert_eq!(serialized["d"], "dee");
}

// ============================================================================
// Flatten + normal nested fields combined
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize, DeriveCarveViewModel, PartialEq)]
pub struct Config {
    pub timeout: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize, DeriveCarveViewModel)]
pub struct Service {
    pub name: String,

    #[carve(flatten)]
    pub metadata: Inner,

    #[carve(nested)]
    pub config: Config,
}

#[test]
fn test_flatten_with_normal_nested() {
    let value = json!({
        "name": "my_service",
        "value": 100,          // from flattened metadata
        "label": "prod",       // from flattened metadata
        "config": {            // normal nested
            "timeout": 30
        }
    });

    let service = Service::from_value(&value).unwrap();

    assert_eq!(service.name, "my_service");
    assert_eq!(service.metadata.value, 100);
    assert_eq!(service.metadata.label, "prod");
    assert_eq!(service.config.timeout, 30);

    // to_value
    let serialized = service.to_value();
    assert_eq!(serialized["name"], "my_service");
    assert_eq!(serialized["value"], 100);
    assert_eq!(serialized["label"], "prod");
    assert_eq!(serialized["config"]["timeout"], 30);
}

// ============================================================================
// Flatten key conflicts (follows serde semantics)
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize, DeriveCarveViewModel, PartialEq)]
pub struct Settings1 {
    pub theme: String,
    pub language: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, DeriveCarveViewModel, PartialEq)]
pub struct Settings2 {
    pub language: String, // CONFLICT: same key as Settings1
    pub region: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, DeriveCarveViewModel)]
pub struct AppConfig {
    pub app_name: String,

    #[carve(flatten)]
    pub settings1: Settings1,

    #[carve(flatten)]
    pub settings2: Settings2, // This will override "language" from settings1
}

#[test]
fn test_flatten_key_conflict_last_wins() {
    // When multiple flattened structs have the same field name,
    // serde's behavior is: last one wins (in struct definition order)
    let value = json!({
        "app_name": "MyApp",
        "theme": "dark",
        "language": "zh", // This will go to settings2.language (last defined)
        "region": "CN"
    });

    let config = AppConfig::from_value(&value).unwrap();

    assert_eq!(config.app_name, "MyApp");
    assert_eq!(config.settings1.theme, "dark");
    // settings1.language gets the value because it's read during settings1 deserialization
    // But in serde's flatten, the LAST field definition wins during serialization
    assert_eq!(config.settings2.language, "zh");
    assert_eq!(config.settings2.region, "CN");
}

#[test]
fn test_flatten_key_conflict_serialization() {
    // Test that serialization follows serde's behavior
    let config = AppConfig {
        app_name: "MyApp".to_string(),
        settings1: Settings1 {
            theme: "dark".to_string(),
            language: "en".to_string(), // This will be OVERRIDDEN in JSON
        },
        settings2: Settings2 {
            language: "zh".to_string(), // This wins (last defined)
            region: "CN".to_string(),
        },
    };

    let serialized = config.to_value();

    assert_eq!(serialized["app_name"], "MyApp");
    assert_eq!(serialized["theme"], "dark");
    assert_eq!(serialized["language"], "zh"); // settings2's value wins
    assert_eq!(serialized["region"], "CN");

    // There should be no duplicate "language" key
    let obj = serialized.as_object().unwrap();
    let language_count = obj.keys().filter(|k| *k == "language").count();
    assert_eq!(language_count, 1, "Should only have one 'language' key");
}

#[test]
fn test_flatten_key_conflict_round_trip() {
    // Round-trip test with conflicting keys
    let original_json = json!({
        "app_name": "MyApp",
        "theme": "light",
        "language": "fr",
        "region": "FR"
    });

    let config = AppConfig::from_value(&original_json).unwrap();
    let serialized = config.to_value();

    // The round-trip should preserve the JSON structure
    assert_eq!(serialized["app_name"], "MyApp");
    assert_eq!(serialized["theme"], "light");
    assert_eq!(serialized["language"], "fr");
    assert_eq!(serialized["region"], "FR");
}

#[test]
fn test_flatten_key_conflict_writer() {
    let mut writer = AppConfig::write();

    writer.app_name("TestApp");

    {
        let mut s1 = writer.settings1();
        s1.theme("dark");
        s1.language("en");
    }

    {
        let mut s2 = writer.settings2();
        s2.language("zh"); // Conflicts with settings1.language
        s2.region("CN");
    }

    let patch = writer.build();
    let result = apply_patch(&json!({}), &patch).unwrap();

    // Both language sets should be in the patch
    // Last one wins in the final JSON
    assert_eq!(result["app_name"], "TestApp");
    assert_eq!(result["theme"], "dark");
    assert_eq!(result["language"], "zh"); // settings2 wins (written last)
    assert_eq!(result["region"], "CN");
}

#[test]
fn test_flatten_partial_conflict() {
    // Only some fields conflict, others are unique
    #[derive(Debug, Clone, Serialize, Deserialize, DeriveCarveViewModel, PartialEq)]
    pub struct PartA {
        pub a_unique: i32,
        pub shared: String,
    }

    #[derive(Debug, Clone, Serialize, Deserialize, DeriveCarveViewModel, PartialEq)]
    pub struct PartB {
        pub b_unique: i32,
        pub shared: String, // Conflicts with PartA
    }

    #[derive(Debug, Clone, Serialize, Deserialize, DeriveCarveViewModel)]
    pub struct Combined {
        #[carve(flatten)]
        pub part_a: PartA,

        #[carve(flatten)]
        pub part_b: PartB,
    }

    let value = json!({
        "a_unique": 1,
        "b_unique": 2,
        "shared": "value"
    });

    let combined = Combined::from_value(&value).unwrap();

    assert_eq!(combined.part_a.a_unique, 1);
    assert_eq!(combined.part_b.b_unique, 2);
    // "shared" goes to both, last wins in serialization
    assert_eq!(combined.part_b.shared, "value");

    // Serialize back
    let serialized = combined.to_value();
    assert_eq!(serialized["a_unique"], 1);
    assert_eq!(serialized["b_unique"], 2);
    assert_eq!(serialized["shared"], "value");
}
