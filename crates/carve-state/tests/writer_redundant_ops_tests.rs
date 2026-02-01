//! Tests for Writer redundant operations behavior.
//!
//! These tests verify that Writer does NOT automatically deduplicate operations,
//! but users can call canonicalize() to optimize the patch if desired.

use carve_state::{apply_patch, AccessorOps};
use carve_state_derive::CarveViewModel as DeriveCarveViewModel;
use serde::{Deserialize, Serialize};
use serde_json::json;

#[derive(Debug, Clone, Serialize, Deserialize, DeriveCarveViewModel)]
pub struct Person {
    pub name: String,
    pub age: u32,
    pub email: Option<String>,
}

// ============================================================================
// Writer allows duplicate operations
// ============================================================================

#[test]
fn test_writer_allows_duplicate_sets() {
    let mut writer = Person::write();

    // Set the same field multiple times
    writer.name("Alice");
    writer.name("Bob");
    writer.name("Charlie");

    let patch = writer.build();

    // Writer should NOT deduplicate automatically
    // All 3 Set operations should be present
    assert_eq!(
        patch.len(),
        3,
        "Writer should generate all operations, not deduplicate"
    );

    // But canonicalize should reduce to just the last one
    let canonical = patch.canonicalize();
    assert_eq!(
        canonical.len(),
        1,
        "Canonicalize should remove redundant operations"
    );

    // Verify semantics are the same
    let doc = json!({});
    let result_original = apply_patch(&doc, &patch).unwrap();
    let result_canonical = apply_patch(&doc, &canonical).unwrap();
    assert_eq!(result_original, result_canonical);
    assert_eq!(result_original["name"], "Charlie");
}

#[test]
fn test_writer_multiple_field_updates() {
    let mut writer = Person::write();

    // Update multiple fields multiple times
    writer.name("Alice");
    writer.age(25);
    writer.name("Bob"); // Duplicate on name
    writer.age(30); // Duplicate on age
    writer.email(Some("bob@example.com".to_string()));

    let patch = writer.build();

    // Should have 5 operations total
    assert_eq!(patch.len(), 5, "All operations should be recorded");

    // Canonicalize should reduce to 3 (last name, last age, email)
    let canonical = patch.canonicalize();
    assert_eq!(canonical.len(), 3, "Should keep only last operation per field");

    // Verify final state is correct
    let doc = json!({});
    let result = apply_patch(&doc, &canonical).unwrap();
    assert_eq!(result["name"], "Bob");
    assert_eq!(result["age"], 30);
    assert_eq!(result["email"], "bob@example.com");
}

#[test]
fn test_writer_interleaved_operations() {
    let mut writer = Person::write();

    // Interleave operations on different fields
    writer.name("Alice");
    writer.age(25);
    writer.name("Bob"); // Back to name
    writer.email(Some("bob@example.com".to_string()));
    writer.age(30); // Back to age

    let patch = writer.build();
    assert_eq!(patch.len(), 5);

    // Canonicalize should preserve order but remove duplicates
    let canonical = patch.canonicalize();
    assert_eq!(canonical.len(), 3);

    // Verify the canonical patch preserves the correct order
    // The order should be: email, name=Bob, age=30
    // because canonicalize preserves relative order of kept operations
    let doc = json!({});
    let result = apply_patch(&doc, &canonical).unwrap();
    assert_eq!(result["name"], "Bob");
    assert_eq!(result["age"], 30);
    assert_eq!(result["email"], "bob@example.com");
}

// ============================================================================
// Accessor allows duplicate operations
// ============================================================================

#[test]
fn test_accessor_allows_duplicate_sets() {
    let doc = json!({
        "name": "Original",
        "age": 20,
        "email": "original@example.com"
    });

    let accessor = Person::access(&doc);

    // Set the same field multiple times
    accessor.name().set("Alice".to_string());
    accessor.name().set("Bob".to_string());
    accessor.name().set("Charlie".to_string());

    let patch = accessor.build();

    // Accessor should NOT deduplicate automatically
    assert_eq!(
        patch.len(),
        3,
        "Accessor should generate all operations, not deduplicate"
    );

    // Canonicalize should reduce to one
    let canonical = patch.canonicalize();
    assert_eq!(canonical.len(), 1);

    // Verify final result
    let result = apply_patch(&doc, &canonical).unwrap();
    assert_eq!(result["name"], "Charlie");
}

#[test]
fn test_accessor_read_then_write_same_field() {
    let doc = json!({
        "name": "Alice",
        "age": 25
    });

    let accessor = Person::access(&doc);

    // Read the value
    let original_name = accessor.name().get().unwrap();
    assert_eq!(original_name, "Alice");

    // Now write to it multiple times
    accessor.name().set("Bob".to_string());
    accessor.name().set("Charlie".to_string());

    let patch = accessor.build();

    // Both set operations should be recorded
    assert_eq!(patch.len(), 2);

    let result = apply_patch(&doc, &patch).unwrap();
    assert_eq!(result["name"], "Charlie");
}

// ============================================================================
// Nested writer operations
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize, DeriveCarveViewModel)]
pub struct Address {
    pub street: String,
    pub city: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, DeriveCarveViewModel)]
pub struct UserWithAddress {
    pub name: String,

    #[carve(nested)]
    pub address: Address,
}

#[test]
fn test_nested_writer_duplicate_operations() {
    let mut writer = UserWithAddress::write();

    writer.name("Alice");

    // Update nested fields multiple times
    {
        let mut addr = writer.address();
        addr.street("123 Main St");
        addr.city("NYC");
    }

    // Update nested again
    {
        let mut addr = writer.address();
        addr.street("456 Oak Ave"); // Duplicate
        addr.city("LA"); // Duplicate
    }

    let patch = writer.build();

    // Should have: 1 name + 2 address.street + 2 address.city = 5 ops
    assert_eq!(
        patch.len(),
        5,
        "Should record all nested operations including duplicates"
    );

    // Canonicalize should reduce to 3
    let canonical = patch.canonicalize();
    assert_eq!(canonical.len(), 3);

    let doc = json!({});
    let result = apply_patch(&doc, &canonical).unwrap();
    assert_eq!(result["name"], "Alice");
    assert_eq!(result["address"]["street"], "456 Oak Ave");
    assert_eq!(result["address"]["city"], "LA");
}

// ============================================================================
// Option<Nested> writer operations (using separate type to avoid conflicts)
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize, DeriveCarveViewModel)]
pub struct Location {
    pub street: String,
    pub city: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, DeriveCarveViewModel)]
pub struct ProfileWithOptionalLocation {
    pub name: String,

    #[carve(nested)]
    pub location: Option<Location>,
}

#[test]
fn test_option_nested_writer_duplicate_operations() {
    let mut writer = ProfileWithOptionalLocation::write();

    writer.name("Bob");

    // Update location fields multiple times
    {
        let mut loc = writer.location();
        loc.street("123 Main");
        loc.city("NYC");
    }

    // Update location again - different values
    {
        let mut loc = writer.location();
        loc.street("456 Oak");
        loc.city("LA");
    }

    let patch = writer.build();

    // Should record all operations
    // 1 name + 2 location.street + 2 location.city = 5 ops
    assert_eq!(
        patch.len(),
        5,
        "Should record all Option<Nested> field updates"
    );

    // Canonicalize should reduce to 3 (name + last street + last city)
    let canonical = patch.canonicalize();
    assert_eq!(canonical.len(), 3);

    let doc = json!({});
    let result = apply_patch(&doc, &canonical).unwrap();
    assert_eq!(result["name"], "Bob");
    assert_eq!(result["location"]["street"], "456 Oak");
    assert_eq!(result["location"]["city"], "LA");
}

// ============================================================================
// Vec operations (should NOT be deduplicated by canonicalize)
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize, DeriveCarveViewModel)]
pub struct Team {
    pub name: String,
    pub members: Vec<String>,
}

#[test]
fn test_vec_operations_not_deduplicated() {
    let mut writer = Team::write();

    writer.name("Engineering");

    // Append multiple items
    writer.members_push("Alice".to_string());
    writer.members_push("Bob".to_string());
    writer.members_push("Charlie".to_string());

    let patch = writer.build();
    assert_eq!(patch.len(), 4); // 1 name + 3 appends

    // Canonicalize should NOT remove array operations
    let canonical = patch.canonicalize();
    assert_eq!(
        canonical.len(),
        4,
        "Array operations should not be deduplicated"
    );

    let doc = json!({});
    let result = apply_patch(&doc, &canonical).unwrap();
    assert_eq!(result["members"].as_array().unwrap().len(), 3);
}

#[test]
fn test_writer_explicit_optimization_workflow() {
    // This test demonstrates the intended workflow:
    // 1. Writer generates all operations (no automatic dedup)
    // 2. User can optionally call canonicalize() to optimize
    // 3. Apply either version - semantics are identical

    let mut writer = Person::write();

    writer.name("v1");
    writer.name("v2");
    writer.name("v3");
    writer.age(10);
    writer.age(20);
    writer.age(30);

    let patch = writer.build();

    // Unoptimized: 6 operations
    assert_eq!(patch.len(), 6);

    // User explicitly optimizes
    let optimized = patch.canonicalize();
    assert_eq!(optimized.len(), 2); // Just last name and last age

    // Both produce same result
    let doc = json!({});
    let result1 = apply_patch(&doc, &patch).unwrap();
    let result2 = apply_patch(&doc, &optimized).unwrap();

    assert_eq!(result1, result2);
    assert_eq!(result1["name"], "v3");
    assert_eq!(result1["age"], 30);

    // The difference is performance (optimized is faster)
    // This is demonstrated in the benchmarks
}
