//! P0 regression tests for structural fixes.
//!
//! Tests covering:
//! - P0-1: FieldKind nested attr preserved through containers
//! - P0-2: Trait-based nested type references (cross-module support)
//! - P0-3: Guard naming with field names (duplicate type support)

use carve_state::{apply_patch, AccessorOps};
use carve_state_derive::CarveViewModel;
use serde::{Deserialize, Serialize};
use serde_json::json;

// ============================================================================
// P0-1: Nested attr works with containers (Option/Vec/Map)
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize, CarveViewModel, PartialEq)]
pub struct Profile {
    pub name: String,
    pub age: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, CarveViewModel)]
pub struct User {
    pub id: String,

    // P0-1: Option<Nested> should work
    #[carve(nested)]
    pub profile: Option<Profile>,

    // P0-1: Vec<Nested> should work
    #[carve(nested)]
    pub friends: Vec<Profile>,
}

#[test]
fn test_option_nested_reader() {
    let doc = json!({
        "id": "user1",
        "profile": {"name": "Alice", "age": 30},
        "friends": []
    });

    let reader = User::read(&doc);

    // Option<Nested> should return Option<Reader>
    let profile_reader = reader.profile().unwrap();
    assert!(profile_reader.is_some());
    let profile = profile_reader.unwrap();
    assert_eq!(profile.name().unwrap(), "Alice");
    assert_eq!(profile.age().unwrap(), 30);
}

#[test]
fn test_option_nested_none() {
    let doc = json!({
        "id": "user2",
        "profile": null,
        "friends": []
    });

    let reader = User::read(&doc);
    let profile_reader = reader.profile().unwrap();
    assert!(profile_reader.is_none());
}

#[test]
fn test_vec_nested_writer() {
    let mut writer = User::write();

    writer.id("user3");

    // Vec<Nested> push should work
    let friend = Profile {
        name: "Bob".to_string(),
        age: 25,
    };
    writer.friends_push(friend);

    let patch = writer.build();
    let doc = json!({});
    let result = apply_patch(&doc, &patch).unwrap();

    assert_eq!(result["id"], "user3");
    assert_eq!(result["friends"][0]["name"], "Bob");
    assert_eq!(result["friends"][0]["age"], 25);
}

#[test]
fn test_vec_nested_accessor() {
    let doc = json!({
        "id": "user4",
        "friends": [
            {"name": "Charlie", "age": 28}
        ]
    });

    let accessor = User::access(&doc);

    // Vec<Nested> should return VecField
    let friends = accessor.friends();
    let vec = friends.get().unwrap();
    assert_eq!(vec.len(), 1);
    assert_eq!(vec[0].name, "Charlie");

    // Push another friend
    friends.push(Profile {
        name: "Dave".to_string(),
        age: 32,
    });

    let patch = accessor.build();
    let result = apply_patch(&doc, &patch).unwrap();
    assert_eq!(result["friends"].as_array().unwrap().len(), 2);
}

// ============================================================================
// P0-2 & P0-3: Same nested type used multiple times (guard naming)
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize, CarveViewModel)]
pub struct Team {
    pub name: String,

    // P0-3: Two fields with same nested type
    // This used to generate duplicate guard names
    #[carve(nested)]
    pub leader: Profile,

    #[carve(nested)]
    pub assistant: Profile,
}

#[test]
fn test_duplicate_nested_type_writer() {
    let mut writer = Team::write();

    writer.name("Engineering");

    // Both leader and assistant should have independent guards
    {
        let mut leader = writer.leader();
        leader.name("Alice");
        leader.age(35);
    }

    {
        let mut assistant = writer.assistant();
        assistant.name("Bob");
        assistant.age(28);
    }

    let patch = writer.build();
    let doc = json!({});
    let result = apply_patch(&doc, &patch).unwrap();

    assert_eq!(result["name"], "Engineering");
    assert_eq!(result["leader"]["name"], "Alice");
    assert_eq!(result["leader"]["age"], 35);
    assert_eq!(result["assistant"]["name"], "Bob");
    assert_eq!(result["assistant"]["age"], 28);
}

#[test]
fn test_duplicate_nested_type_accessor() {
    let doc = json!({
        "name": "Marketing",
        "leader": {"name": "Charlie", "age": 40},
        "assistant": {"name": "Dave", "age": 30}
    });

    let accessor = Team::access(&doc);

    // Both fields should be independently accessible
    {
        let leader = accessor.leader();
        leader.set_age(41);
    }

    {
        let assistant = accessor.assistant();
        assistant.set_age(31);
    }

    let patch = accessor.build();
    let result = apply_patch(&doc, &patch).unwrap();

    assert_eq!(result["leader"]["age"], 41);
    assert_eq!(result["assistant"]["age"], 31);
}

// ============================================================================
// P0-2: Cross-module nested (trait-based references)
// ============================================================================

// Simulate cross-module scenario
mod models {
    use super::*;

    #[derive(Debug, Clone, Serialize, Deserialize, CarveViewModel, PartialEq)]
    pub struct Address {
        pub street: String,
        pub city: String,
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, CarveViewModel)]
pub struct Company {
    pub name: String,

    // P0-2: Nested type from another module
    // This used to fail with "AddressReader not found"
    #[carve(nested)]
    pub headquarters: models::Address,
}

#[test]
fn test_cross_module_nested_reader() {
    let doc = json!({
        "name": "Acme Corp",
        "headquarters": {"street": "123 Main St", "city": "Springfield"}
    });

    let reader = Company::read(&doc);
    let hq = reader.headquarters();

    assert_eq!(hq.street().unwrap(), "123 Main St");
    assert_eq!(hq.city().unwrap(), "Springfield");
}

#[test]
fn test_cross_module_nested_writer() {
    let mut writer = Company::write();

    writer.name("Tech Inc");
    {
        let mut hq = writer.headquarters();
        hq.street("456 Oak Ave");
        hq.city("Portland");
    }

    let patch = writer.build();
    let doc = json!({});
    let result = apply_patch(&doc, &patch).unwrap();

    assert_eq!(result["name"], "Tech Inc");
    assert_eq!(result["headquarters"]["street"], "456 Oak Ave");
    assert_eq!(result["headquarters"]["city"], "Portland");
}

// ============================================================================
// P0-1 + P0-3: Complex nested containers
// ============================================================================

use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize, CarveViewModel)]
pub struct Organization {
    pub name: String,

    // Map with nested value type
    #[carve(nested)]
    pub departments: HashMap<String, Profile>,
}

#[test]
fn test_map_nested_value_accessor() {
    let doc = json!({
        "name": "BigCorp",
        "departments": {
            "engineering": {"name": "Alice", "age": 35},
            "sales": {"name": "Bob", "age": 30}
        }
    });

    let accessor = Organization::access(&doc);

    // Map field should work
    let depts = accessor.departments();
    let map = depts.get().unwrap();
    assert_eq!(map.len(), 2);
    assert_eq!(map.get("engineering").unwrap().name, "Alice");

    // Insert new department
    depts.insert(
        "marketing",
        Profile {
            name: "Charlie".to_string(),
            age: 28,
        },
    );

    let patch = accessor.build();
    let result = apply_patch(&doc, &patch).unwrap();

    assert_eq!(result["departments"]["marketing"]["name"], "Charlie");
}

// ============================================================================
// Verify guards are properly scoped (compile-time test)
// ============================================================================

// This test verifies that guard types don't leak/conflict
// If guards were still named by type (ProfileWriterGuard),
// this would fail to compile with "duplicate definition"
#[test]
fn test_guards_are_field_scoped() {
    // Team has two Profile fields with separate guards
    let mut team_writer = Team::write();
    let _leader_guard = team_writer.leader(); // leaderWriterGuard
    drop(_leader_guard);
    let _assistant_guard = team_writer.assistant(); // assistantWriterGuard

    // If guards had same name, this would be a compile error
    // The fact that this compiles proves P0-3 fix works
}

// ============================================================================
// FIXED: Option<Nested> accessor now correctly merges ops via Guard
// ============================================================================

#[test]
fn test_option_nested_accessor_merges_ops_correctly() {
    // This test verifies that Option<Nested> accessor modifications are now
    // correctly merged back to the parent via the Guard pattern with Drop.

    let doc = json!({
        "id": "user1",
        "profile": {"name": "Alice", "age": 30},
        "friends": []
    });

    let accessor = User::access(&doc);

    // Modify the optional nested field using Guard
    if let Some(mut profile_guard) = accessor.profile().unwrap() {
        // Guard holds parent_ops reference and nested accessor
        // Modifications go into the nested accessor's ops
        profile_guard.set_name("Bob".to_string());
        // When guard is dropped, ops are drained and merged to parent
    }

    // Build patch from parent accessor
    let patch = accessor.build();

    // Apply patch
    let result = apply_patch(&doc, &patch).unwrap();

    // FIXED: The name change is now preserved via Guard Drop merging
    assert_eq!(result["profile"]["name"], "Bob");
    assert_eq!(result["profile"]["age"], 30); // Other fields unchanged
}

#[test]
fn test_direct_nested_accessor_works_correctly() {
    // This test shows that direct nested (non-Option) works correctly
    // because it uses Guard pattern with Drop merging.

    let doc = json!({
        "name": "Acme Corp",
        "headquarters": {"street": "123 Main St", "city": "Springfield"}
    });

    let accessor = Company::access(&doc);

    // Direct nested uses Guard, so ops are merged on Drop
    {
        let mut hq = accessor.headquarters();
        hq.set_street("456 Oak Ave".to_string());
    } // Guard dropped, ops merged

    let patch = accessor.build();
    let result = apply_patch(&doc, &patch).unwrap();

    // This works correctly
    assert_eq!(result["headquarters"]["street"], "456 Oak Ave");
}
