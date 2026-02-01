//! Advanced scenario tests for carve-state.
//!
//! Tests covering complex nested types, error recovery, special values,
//! framework integration patterns, and edge cases.

use carve_state::{
    apply_patch, path, CarveViewModel, JsonWriter, Op, Patch, Path, TrackedPatch, WriterOps,
};
use carve_state_derive::CarveViewModel;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::BTreeMap;

// ============================================================================
// Complex Nested Types
// ============================================================================

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, CarveViewModel)]
pub struct Address {
    pub street: String,
    pub city: String,
    pub zip: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, CarveViewModel)]
pub struct Profile {
    pub bio: String,
    pub website: Option<String>,
}

// NOTE: Option<NestedType> with #[carve(nested)] is not yet fully supported.
// This is tracked as a future enhancement.
// For now, Option<NestedType> can be used without the nested attribute,
// but it will be serialized/deserialized as a whole value.

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, CarveViewModel)]
pub struct UserWithOptionalProfile {
    pub name: String,
    pub profile: Option<Profile>,  // Without #[carve(nested)] - treated as primitive
}

#[test]
fn test_option_struct_as_value() {
    let doc = json!({
        "name": "Alice",
        "profile": {
            "bio": "Developer",
            "website": "https://alice.dev"
        }
    });

    let reader = UserWithOptionalProfile::read(&doc);
    assert_eq!(reader.name().unwrap(), "Alice");

    // Option<Profile> is deserialized as a whole value
    let profile = reader.profile().unwrap();
    assert!(profile.is_some());
    let p = profile.unwrap();
    assert_eq!(p.bio, "Developer");
    assert_eq!(p.website, Some("https://alice.dev".to_string()));
}

#[test]
fn test_option_struct_none() {
    let doc = json!({
        "name": "Bob",
        "profile": null
    });

    let reader = UserWithOptionalProfile::read(&doc);
    assert_eq!(reader.name().unwrap(), "Bob");
    assert_eq!(reader.profile().unwrap(), None);
}

#[test]
fn test_option_struct_missing() {
    let doc = json!({
        "name": "Charlie"
    });

    let reader = UserWithOptionalProfile::read(&doc);
    assert_eq!(reader.name().unwrap(), "Charlie");
    assert_eq!(reader.profile().unwrap(), None);
}

// Deep nesting (3+ levels)
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, CarveViewModel)]
pub struct Avatar {
    pub url: String,
    pub size: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, CarveViewModel)]
pub struct DeepProfile {
    pub bio: String,
    #[carve(nested)]
    pub avatar: Avatar,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, CarveViewModel)]
pub struct DeepUser {
    pub name: String,
    #[carve(nested)]
    pub profile: DeepProfile,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, CarveViewModel)]
pub struct DeepApp {
    pub version: String,
    #[carve(nested)]
    pub user: DeepUser,
}

#[test]
fn test_deep_nesting_read() {
    let doc = json!({
        "version": "1.0",
        "user": {
            "name": "Alice",
            "profile": {
                "bio": "Developer",
                "avatar": {
                    "url": "https://example.com/avatar.png",
                    "size": 128
                }
            }
        }
    });

    let reader = DeepApp::read(&doc);
    assert_eq!(reader.version().unwrap(), "1.0");
    assert_eq!(reader.user().name().unwrap(), "Alice");
    assert_eq!(reader.user().profile().bio().unwrap(), "Developer");
    assert_eq!(
        reader.user().profile().avatar().url().unwrap(),
        "https://example.com/avatar.png"
    );
    assert_eq!(reader.user().profile().avatar().size().unwrap(), 128);
}

#[test]
fn test_deep_nesting_write() {
    let mut writer = DeepApp::write();
    writer.version("2.0");
    writer.user().name("Bob");
    writer.user().profile().bio("Engineer");
    writer.user().profile().avatar().url("https://new-avatar.png");
    writer.user().profile().avatar().size(256);

    let patch = writer.build();

    let doc = json!({});
    let result = apply_patch(&doc, &patch).unwrap();

    assert_eq!(result["version"], "2.0");
    assert_eq!(result["user"]["name"], "Bob");
    assert_eq!(result["user"]["profile"]["bio"], "Engineer");
    assert_eq!(result["user"]["profile"]["avatar"]["url"], "https://new-avatar.png");
    assert_eq!(result["user"]["profile"]["avatar"]["size"], 256);
}

#[test]
fn test_deep_nesting_get() {
    let doc = json!({
        "version": "1.0",
        "user": {
            "name": "Alice",
            "profile": {
                "bio": "Dev",
                "avatar": {
                    "url": "http://x.com/a.png",
                    "size": 64
                }
            }
        }
    });

    let reader = DeepApp::read(&doc);
    let app = reader.get().unwrap();

    assert_eq!(app.version, "1.0");
    assert_eq!(app.user.name, "Alice");
    assert_eq!(app.user.profile.bio, "Dev");
    assert_eq!(app.user.profile.avatar.url, "http://x.com/a.png");
    assert_eq!(app.user.profile.avatar.size, 64);
}

// ============================================================================
// Type Mismatch and Error Recovery
// ============================================================================

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, CarveViewModel)]
pub struct TypedStruct {
    pub count: u32,
    pub name: String,
    pub active: bool,
}

#[test]
fn test_type_mismatch_string_for_number() {
    let doc = json!({
        "count": "not a number",
        "name": "test",
        "active": true
    });

    let reader = TypedStruct::read(&doc);
    // Reading mismatched type should return error
    let result = reader.count();
    assert!(result.is_err());

    // Other fields should still work
    assert_eq!(reader.name().unwrap(), "test");
    assert_eq!(reader.active().unwrap(), true);
}

#[test]
fn test_type_mismatch_number_for_string() {
    let doc = json!({
        "count": 42,
        "name": 12345,
        "active": true
    });

    let reader = TypedStruct::read(&doc);
    assert_eq!(reader.count().unwrap(), 42);
    // Number can't be deserialized as String
    assert!(reader.name().is_err());
}

#[test]
fn test_type_mismatch_get_fails() {
    let doc = json!({
        "count": "invalid",
        "name": "test",
        "active": true
    });

    let reader = TypedStruct::read(&doc);
    // get() should fail if any field fails deserialization
    let result = reader.get();
    assert!(result.is_err());
}

#[test]
fn test_partial_data_with_defaults() {
    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize, CarveViewModel)]
    struct WithDefaults {
        name: String,
        #[carve(default = "0")]
        count: u32,
        #[carve(default = "false")]
        enabled: bool,
    }

    let doc = json!({"name": "test"});

    let reader = WithDefaults::read(&doc);
    assert_eq!(reader.name().unwrap(), "test");
    assert_eq!(reader.count().unwrap(), 0);
    assert_eq!(reader.enabled().unwrap(), false);
}

// ============================================================================
// Special Value Handling
// ============================================================================

#[test]
fn test_null_vs_missing_vs_empty() {
    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize, CarveViewModel)]
    struct NullTest {
        value: Option<String>,
    }

    // Explicit null
    let doc_null = json!({"value": null});
    let reader = NullTest::read(&doc_null);
    assert_eq!(reader.value().unwrap(), None);

    // Missing field
    let doc_missing = json!({});
    let reader = NullTest::read(&doc_missing);
    assert_eq!(reader.value().unwrap(), None);

    // Empty string (not None!)
    let doc_empty = json!({"value": ""});
    let reader = NullTest::read(&doc_empty);
    assert_eq!(reader.value().unwrap(), Some("".to_string()));
}

#[test]
fn test_unicode_chinese() {
    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize, CarveViewModel)]
    struct UnicodeStruct {
        name: String,
        description: String,
    }

    let doc = json!({
        "name": "张三",
        "description": "这是一个测试"
    });

    let reader = UnicodeStruct::read(&doc);
    assert_eq!(reader.name().unwrap(), "张三");
    assert_eq!(reader.description().unwrap(), "这是一个测试");

    // Write unicode
    let mut writer = UnicodeStruct::write();
    writer.name("李四");
    writer.description("中文描述");
    let patch = writer.build();

    let result = apply_patch(&json!({}), &patch).unwrap();
    assert_eq!(result["name"], "李四");
    assert_eq!(result["description"], "中文描述");
}

#[test]
fn test_unicode_emoji() {
    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize, CarveViewModel)]
    struct EmojiStruct {
        status: String,
    }

    let doc = json!({"status": "Happy 😊🎉"});
    let reader = EmojiStruct::read(&doc);
    assert_eq!(reader.status().unwrap(), "Happy 😊🎉");
}

#[test]
fn test_special_json_keys() {
    // Keys with dots, slashes, etc.
    let doc = json!({
        "foo.bar": 1,
        "a/b/c": 2,
        "key with spaces": 3,
        "special!@#$%": 4
    });

    // Use JsonWriter for dynamic keys
    let mut writer = JsonWriter::new();
    writer.set(path!("foo.bar"), json!(10));
    writer.set(path!("a/b/c"), json!(20));

    let patch = writer.build();
    let result = apply_patch(&doc, &patch).unwrap();

    assert_eq!(result["foo.bar"], 10);
    assert_eq!(result["a/b/c"], 20);
}

#[test]
fn test_empty_string_key() {
    let doc = json!({"": "empty key value"});

    let value = carve_state::get_at_path(&doc, &path!(""));
    assert_eq!(value, Some(&json!("empty key value")));
}

// ============================================================================
// TrackedPatch Tests
// ============================================================================

#[test]
fn test_tracked_patch_basic() {
    let patch = Patch::new()
        .with_op(Op::set(path!("x"), json!(1)))
        .with_op(Op::set(path!("y"), json!(2)));

    let tracked = TrackedPatch::new(patch.clone());
    assert_eq!(tracked.patch, patch);
    assert!(tracked.id.is_none());
    assert!(tracked.timestamp.is_none());
    assert!(tracked.source.is_none());
}

#[test]
fn test_tracked_patch_with_metadata() {
    let patch = Patch::new().with_op(Op::set(path!("x"), json!(1)));

    let tracked = TrackedPatch::new(patch)
        .with_id("tx-12345")
        .with_timestamp(1234567890)
        .with_source("user-action");

    assert_eq!(tracked.id, Some("tx-12345".to_string()));
    assert_eq!(tracked.timestamp, Some(1234567890));
    assert_eq!(tracked.source, Some("user-action".to_string()));
}

#[test]
fn test_tracked_patch_serde() {
    let patch = Patch::new().with_op(Op::set(path!("x"), json!(1)));

    let tracked = TrackedPatch::new(patch)
        .with_id("tx-001")
        .with_timestamp(1000)
        .with_source("test");

    let json_str = serde_json::to_string(&tracked).unwrap();
    let restored: TrackedPatch = serde_json::from_str(&json_str).unwrap();

    assert_eq!(tracked.id, restored.id);
    assert_eq!(tracked.timestamp, restored.timestamp);
    assert_eq!(tracked.source, restored.source);
    assert_eq!(tracked.patch.len(), restored.patch.len());
}

// ============================================================================
// Framework Integration Pattern Tests
// ============================================================================

/// Simulated framework that manages state and components
struct TestFramework {
    state: serde_json::Value,
    patches: Vec<Patch>,
}

impl TestFramework {
    fn new(initial_state: serde_json::Value) -> Self {
        Self {
            state: initial_state,
            patches: Vec::new(),
        }
    }

    /// Process a component generically using CarveViewModel
    fn process<T: CarveViewModel>(
        &mut self,
        base: Path,
        handler: impl FnOnce(T::Reader<'_>, &mut T::Writer),
    ) -> &Patch {
        let reader = T::reader(&self.state, base.clone());
        let mut writer = T::writer(base);

        handler(reader, &mut writer);

        let patch = writer.into_patch();

        if !patch.is_empty() {
            self.state = apply_patch(&self.state, &patch).unwrap();
        }

        self.patches.push(patch);
        self.patches.last().unwrap()
    }

    fn state(&self) -> &serde_json::Value {
        &self.state
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, CarveViewModel)]
pub struct Counter {
    pub value: i32,
    pub label: String,
}

#[test]
fn test_framework_single_component() {
    let mut framework = TestFramework::new(json!({
        "value": 0,
        "label": "Counter"
    }));

    framework.process::<Counter>(Path::root(), |reader, writer| {
        let current = reader.value().unwrap_or(0);
        writer.value(current + 1);
    });

    assert_eq!(framework.state()["value"], 1);
}

#[test]
fn test_framework_multiple_updates() {
    let mut framework = TestFramework::new(json!({
        "value": 0,
        "label": "Counter"
    }));

    // Multiple updates
    for _ in 0..5 {
        framework.process::<Counter>(Path::root(), |reader, writer| {
            let current = reader.value().unwrap_or(0);
            writer.value(current + 1);
        });
    }

    assert_eq!(framework.state()["value"], 5);
}

#[test]
fn test_framework_nested_path() {
    let mut framework = TestFramework::new(json!({
        "counters": {
            "main": {
                "value": 10,
                "label": "Main Counter"
            }
        }
    }));

    framework.process::<Counter>(path!("counters", "main"), |reader, writer| {
        let current = reader.value().unwrap_or(0);
        writer.value(current * 2);
        writer.label("Updated");
    });

    assert_eq!(framework.state()["counters"]["main"]["value"], 20);
    assert_eq!(framework.state()["counters"]["main"]["label"], "Updated");
}

#[test]
fn test_framework_collect_patches() {
    let mut framework = TestFramework::new(json!({"value": 0, "label": "Test"}));

    framework.process::<Counter>(Path::root(), |_, writer| {
        writer.value(1);
    });

    framework.process::<Counter>(Path::root(), |_, writer| {
        writer.value(2);
    });

    // All patches are collected
    assert_eq!(framework.patches.len(), 2);

    // Patches can be serialized for persistence/sync
    let all_patches: Vec<_> = framework
        .patches
        .iter()
        .map(|p| serde_json::to_string(p).unwrap())
        .collect();

    assert_eq!(all_patches.len(), 2);
}

// ============================================================================
// Concurrent Write Behavior Tests
// ============================================================================

#[test]
fn test_multiple_writes_same_field() {
    let mut writer = Counter::write();

    // Write same field multiple times
    writer.value(1);
    writer.value(2);
    writer.value(3);

    let patch = writer.build();

    // Each write creates a separate op
    assert_eq!(patch.len(), 3);

    // Last write wins when applied
    let doc = json!({"value": 0, "label": "test"});
    let result = apply_patch(&doc, &patch).unwrap();
    assert_eq!(result["value"], 3);
}

#[test]
fn test_write_then_delete() {
    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize, CarveViewModel)]
    struct DeleteTest {
        name: String,
        age: u32,
    }

    let mut writer = DeleteTest::write();
    writer.name("Alice");
    writer.age(30);
    writer.delete_name(); // Delete after write

    let patch = writer.build();

    let doc = json!({});
    let result = apply_patch(&doc, &patch).unwrap();

    // Name was deleted after being set
    assert!(result.get("name").is_none());
    assert_eq!(result["age"], 30);
}

#[test]
fn test_nested_multiple_writes() {
    let mut writer = DeepApp::write();

    // Multiple writes to nested fields
    writer.user().name("First");
    writer.user().name("Second");
    writer.user().profile().bio("Bio 1");
    writer.user().profile().bio("Bio 2");

    let patch = writer.build();

    let doc = json!({});
    let result = apply_patch(&doc, &patch).unwrap();

    // Last writes win
    assert_eq!(result["user"]["name"], "Second");
    assert_eq!(result["user"]["profile"]["bio"], "Bio 2");
}

// ============================================================================
// Vec and Map with Complex Types
// ============================================================================

#[test]
fn test_vec_of_numbers() {
    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize, CarveViewModel)]
    struct NumericVec {
        integers: Vec<i64>,
        floats: Vec<f64>,
    }

    let doc = json!({
        "integers": [1, 2, 3, -4, 5],
        "floats": [1.1, 2.2, 3.3]
    });

    let reader = NumericVec::read(&doc);
    assert_eq!(reader.integers().unwrap(), vec![1, 2, 3, -4, 5]);
    assert_eq!(reader.floats().unwrap(), vec![1.1, 2.2, 3.3]);
}

#[test]
fn test_map_with_complex_values() {
    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize, CarveViewModel)]
    struct ComplexMap {
        scores: BTreeMap<String, i32>,
        metadata: BTreeMap<String, String>,
    }

    let doc = json!({
        "scores": {"alice": 100, "bob": 85, "charlie": 92},
        "metadata": {"version": "1.0", "author": "test"}
    });

    let reader = ComplexMap::read(&doc);

    let scores = reader.scores().unwrap();
    assert_eq!(scores.get("alice"), Some(&100));
    assert_eq!(scores.get("bob"), Some(&85));

    let metadata = reader.metadata().unwrap();
    assert_eq!(metadata.get("version"), Some(&"1.0".to_string()));
}

#[test]
fn test_empty_collections() {
    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize, CarveViewModel)]
    struct EmptyCollections {
        items: Vec<String>,
        data: BTreeMap<String, i32>,
    }

    let doc = json!({
        "items": [],
        "data": {}
    });

    let reader = EmptyCollections::read(&doc);
    assert!(reader.items().unwrap().is_empty());
    assert!(reader.data().unwrap().is_empty());
}

// ============================================================================
// Rename with Special Cases
// ============================================================================

#[test]
fn test_rename_to_json_keywords() {
    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize, CarveViewModel)]
    struct JsonKeywords {
        #[carve(rename = "type")]
        kind: String,
        #[carve(rename = "class")]
        category: String,
    }

    let doc = json!({
        "type": "document",
        "class": "important"
    });

    let reader = JsonKeywords::read(&doc);
    assert_eq!(reader.kind().unwrap(), "document");
    assert_eq!(reader.category().unwrap(), "important");

    let mut writer = JsonKeywords::write();
    writer.kind("file");
    writer.category("normal");

    let patch = writer.build();
    let result = apply_patch(&json!({}), &patch).unwrap();

    assert_eq!(result["type"], "file");
    assert_eq!(result["class"], "normal");
}

// ============================================================================
// Large Data Performance Sanity Check
// ============================================================================

#[test]
fn test_large_array() {
    let large_array: Vec<i32> = (0..1000).collect();
    let doc = json!({"numbers": large_array});

    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize, CarveViewModel)]
    struct LargeArray {
        numbers: Vec<i32>,
    }

    let reader = LargeArray::read(&doc);
    let numbers = reader.numbers().unwrap();
    assert_eq!(numbers.len(), 1000);
    assert_eq!(numbers[0], 0);
    assert_eq!(numbers[999], 999);
}

#[test]
fn test_many_fields_struct() {
    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize, CarveViewModel)]
    struct ManyFields {
        f01: String, f02: String, f03: String, f04: String, f05: String,
        f06: String, f07: String, f08: String, f09: String, f10: String,
    }

    let doc = json!({
        "f01": "1", "f02": "2", "f03": "3", "f04": "4", "f05": "5",
        "f06": "6", "f07": "7", "f08": "8", "f09": "9", "f10": "10"
    });

    let reader = ManyFields::read(&doc);
    assert_eq!(reader.f01().unwrap(), "1");
    assert_eq!(reader.f10().unwrap(), "10");

    let s = reader.get().unwrap();
    assert_eq!(s.f05, "5");
}
