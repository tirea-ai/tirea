//! P1-4/P1-5: Error message improvements tests
//!
//! Tests that verify:
//! - P1-4: from_value errors include path information
//! - P1-5: Reader distinguishes missing vs null for Option fields

use carve_state::{CarveError, CarveViewModel};
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
// P1-4: from_value provides path in errors
// ============================================================================

#[test]
fn test_from_value_missing_field_error_has_path() {
    let value = json!({
        "name": "Alice"
        // age is missing
    });

    let result = Person::from_value(&value);
    assert!(result.is_err());

    match result.unwrap_err() {
        CarveError::PathNotFound { path } => {
            // Error should include the field path (JSON Pointer format: $.fieldname)
            assert_eq!(path.to_string(), "$.age");
        }
        other => panic!("Expected PathNotFound, got: {:?}", other),
    }
}

#[test]
fn test_from_value_type_mismatch_error_has_path() {
    let value = json!({
        "name": "Bob",
        "age": "not a number" // Wrong type
    });

    let result = Person::from_value(&value);
    assert!(result.is_err());

    match result.unwrap_err() {
        CarveError::TypeMismatch { path, expected, found } => {
            assert_eq!(path.to_string(), "$.age");
            // After fix: expected should be field type (u32), not struct type (Person)
            assert!(expected.contains("u32"), "Expected should contain field type 'u32', got: {}", expected);
            assert_eq!(found, "string"); // Value type included
        }
        other => panic!("Expected TypeMismatch, got: {:?}", other),
    }
}

#[test]
fn test_from_value_null_for_required_field() {
    let value = json!({
        "name": "Charlie",
        "age": null // Null for required field
    });

    let result = Person::from_value(&value);
    assert!(result.is_err());

    match result.unwrap_err() {
        CarveError::TypeMismatch { path, .. } => {
            assert_eq!(path.to_string(), "$.age");
        }
        other => panic!("Expected TypeMismatch, got: {:?}", other),
    }
}

// ============================================================================
// P1-5: Reader Option semantics - distinguish missing/null/present
// ============================================================================

#[test]
fn test_reader_option_field_missing() {
    let doc = json!({
        "name": "Alice",
        "age": 30
        // email is missing
    });

    let reader = Person::read(&doc);

    // Missing field should return Ok(None)
    let email = reader.email().unwrap();
    assert_eq!(email, None);
}

#[test]
fn test_reader_option_field_null() {
    let doc = json!({
        "name": "Bob",
        "age": 25,
        "email": null // Explicitly null
    });

    let reader = Person::read(&doc);

    // Null field should return Ok(None)
    let email = reader.email().unwrap();
    assert_eq!(email, None);
}

#[test]
fn test_reader_option_field_present() {
    let doc = json!({
        "name": "Charlie",
        "age": 35,
        "email": "charlie@example.com"
    });

    let reader = Person::read(&doc);

    // Present field should return Ok(Some(value))
    let email = reader.email().unwrap();
    assert_eq!(email, Some("charlie@example.com".to_string()));
}

#[test]
fn test_reader_option_field_type_mismatch() {
    let doc = json!({
        "name": "Dave",
        "age": 40,
        "email": 12345 // Wrong type
    });

    let reader = Person::read(&doc);

    // Type mismatch should return Err with path info
    let result = reader.email();
    assert!(result.is_err());

    match result.unwrap_err() {
        CarveError::TypeMismatch { path, expected, found } => {
            assert_eq!(path.to_string(), "$.email");
            assert!(expected.contains("Option")); // Type includes Option
            assert_eq!(found, "number");
        }
        other => panic!("Expected TypeMismatch, got: {:?}", other),
    }
}

#[test]
fn test_reader_required_field_missing() {
    let doc = json!({
        "name": "Eve"
        // age is missing
    });

    let reader = Person::read(&doc);

    // Required field missing should return PathNotFound
    let result = reader.age();
    assert!(result.is_err());

    match result.unwrap_err() {
        CarveError::PathNotFound { path } => {
            assert_eq!(path.to_string(), "$.age");
        }
        other => panic!("Expected PathNotFound, got: {:?}", other),
    }
}

#[test]
fn test_reader_required_field_type_mismatch() {
    let doc = json!({
        "name": "Frank",
        "age": "thirty" // Wrong type
    });

    let reader = Person::read(&doc);

    let result = reader.age();
    assert!(result.is_err());

    match result.unwrap_err() {
        CarveError::TypeMismatch { path, expected, found } => {
            assert_eq!(path.to_string(), "$.age");
            assert!(expected.contains("u32"));
            assert_eq!(found, "string");
        }
        other => panic!("Expected TypeMismatch, got: {:?}", other),
    }
}

// ============================================================================
// Nested struct error messages
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize, DeriveCarveViewModel)]
pub struct Address {
    pub street: String,
    pub city: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, DeriveCarveViewModel)]
pub struct Company {
    pub name: String,

    #[carve(nested)]
    pub address: Address,
}

#[test]
fn test_nested_from_value_error_has_full_path() {
    let value = json!({
        "name": "Acme Corp",
        "address": {
            "street": "123 Main St"
            // city is missing in nested struct
        }
    });

    // Direct Address::from_value should fail with path "$.city"
    let addr_value = value.get("address").unwrap();
    let addr_result = Address::from_value(addr_value);
    assert!(addr_result.is_err());

    match addr_result.unwrap_err() {
        CarveError::PathNotFound { path } => {
            assert_eq!(path.to_string(), "$.city");
        }
        other => panic!("Expected PathNotFound, got: {:?}", other),
    }

    // Problem 3 fix: Company::from_value should have FULL path "$.address.city"
    let company_result = Company::from_value(&value);
    assert!(company_result.is_err());

    match company_result.unwrap_err() {
        CarveError::PathNotFound { path } => {
            assert_eq!(
                path.to_string(),
                "$.address.city",
                "Nested error should have full path from root"
            );
        }
        other => panic!("Expected PathNotFound with full path, got: {:?}", other),
    }
}

#[test]
fn test_nested_type_mismatch_has_full_path() {
    let value = json!({
        "name": "Acme Corp",
        "address": {
            "street": "123 Main St",
            "city": 12345 // Wrong type
        }
    });

    let result = Company::from_value(&value);
    assert!(result.is_err());

    match result.unwrap_err() {
        CarveError::TypeMismatch { path, expected, found } => {
            assert_eq!(
                path.to_string(),
                "$.address.city",
                "Nested type error should have full path"
            );
            assert!(expected.contains("String") || expected.contains("str"));
            assert_eq!(found, "number");
        }
        other => panic!("Expected TypeMismatch with full path, got: {:?}", other),
    }
}

#[test]
fn test_option_nested_missing_vs_null() {
    // Test Option<Nested> handling
    #[derive(Debug, Clone, Serialize, Deserialize, DeriveCarveViewModel)]
    pub struct Profile {
        pub bio: String,
    }

    #[derive(Debug, Clone, Serialize, Deserialize, DeriveCarveViewModel)]
    pub struct User {
        pub name: String,

        #[carve(nested)]
        pub profile: Option<Profile>,
    }

    // Missing optional nested field
    let value1 = json!({
        "name": "Alice"
    });
    let user1 = User::from_value(&value1).unwrap();
    assert_eq!(user1.name, "Alice");
    assert!(user1.profile.is_none());

    // Null optional nested field
    let value2 = json!({
        "name": "Bob",
        "profile": null
    });
    let user2 = User::from_value(&value2).unwrap();
    assert_eq!(user2.name, "Bob");
    assert!(user2.profile.is_none());

    // Present optional nested field
    let value3 = json!({
        "name": "Charlie",
        "profile": {
            "bio": "Developer"
        }
    });
    let user3 = User::from_value(&value3).unwrap();
    assert_eq!(user3.name, "Charlie");
    assert!(user3.profile.is_some());
    assert_eq!(user3.profile.unwrap().bio, "Developer");

    // Nested field with error should propagate full path
    let value4 = json!({
        "name": "Dave",
        "profile": {
            // bio missing
        }
    });
    let result = User::from_value(&value4);
    assert!(result.is_err());

    match result.unwrap_err() {
        CarveError::PathNotFound { path } => {
            assert_eq!(
                path.to_string(),
                "$.profile.bio",
                "Nested optional error should have full path"
            );
        }
        other => panic!("Expected PathNotFound with full path, got: {:?}", other),
    }
}

// ============================================================================
// Problem 4: default attribute error handling
// ============================================================================

#[test]
fn test_default_field_missing_uses_default() {
    // Field with default should use default when missing
    #[derive(Debug, Clone, Serialize, Deserialize, DeriveCarveViewModel)]
    pub struct Config {
        pub name: String,

        #[carve(default = "42")]
        pub timeout: i32,
    }

    let value = json!({
        "name": "test"
        // timeout is missing
    });

    let result = Config::from_value(&value);
    assert!(result.is_ok());
    let config = result.unwrap();
    assert_eq!(config.name, "test");
    assert_eq!(config.timeout, 42); // Should use default
}

#[test]
fn test_default_field_present_uses_value() {
    // Field with default should use actual value when present
    #[derive(Debug, Clone, Serialize, Deserialize, DeriveCarveViewModel)]
    pub struct Config {
        pub name: String,

        #[carve(default = "42")]
        pub timeout: i32,
    }

    let value = json!({
        "name": "test",
        "timeout": 100
    });

    let result = Config::from_value(&value);
    assert!(result.is_ok());
    let config = result.unwrap();
    assert_eq!(config.name, "test");
    assert_eq!(config.timeout, 100); // Should use actual value
}

#[test]
fn test_default_field_type_mismatch_errors() {
    // Problem 4 fix: field with default should ERROR on type mismatch
    // not silently use default (which would hide data quality issues)
    #[derive(Debug, Clone, Serialize, Deserialize, DeriveCarveViewModel)]
    pub struct Config {
        pub name: String,

        #[carve(default = "42")]
        pub timeout: i32,
    }

    let value = json!({
        "name": "test",
        "timeout": "not_a_number" // Wrong type
    });

    let result = Config::from_value(&value);
    assert!(result.is_err(), "Should error on type mismatch, not use default");

    match result.unwrap_err() {
        CarveError::TypeMismatch { path, expected, found } => {
            assert_eq!(path.to_string(), "$.timeout");
            assert!(expected.contains("i32"), "Expected should be field type");
            assert_eq!(found, "string");
        }
        other => panic!("Expected TypeMismatch, got: {:?}", other),
    }
}

#[test]
fn test_default_field_null_is_type_error() {
    // For non-Option types, null is a type mismatch, not "missing"
    // If you want null → default, use Option<T> or #[serde(default)]
    #[derive(Debug, Clone, Serialize, Deserialize, DeriveCarveViewModel)]
    pub struct Config {
        pub name: String,

        #[carve(default = "42")]
        pub timeout: i32,
    }

    let value = json!({
        "name": "test",
        "timeout": null
    });

    // null for non-Option type should be a type error
    let result = Config::from_value(&value);
    assert!(result.is_err(), "null should be type error for non-Option field");

    match result.unwrap_err() {
        CarveError::TypeMismatch { path, expected, found } => {
            assert_eq!(path.to_string(), "$.timeout");
            assert!(expected.contains("i32"));
            assert_eq!(found, "null");
        }
        other => panic!("Expected TypeMismatch for null, got: {:?}", other),
    }
}

// ============================================================================
// Error message clarity verification
// ============================================================================

#[test]
fn test_error_display_includes_path() {
    let value = json!({
        "name": "Test"
        // age missing
    });

    let result = Person::from_value(&value);
    let error = result.unwrap_err();
    let error_str = error.to_string();

    // Error message should mention the field name
    assert!(
        error_str.contains("age"),
        "Error should mention field 'age': {}",
        error_str
    );
}

#[test]
fn test_type_mismatch_error_display() {
    let value = json!({
        "name": "Test",
        "age": "wrong"
    });

    let result = Person::from_value(&value);
    let error = result.unwrap_err();
    let error_str = error.to_string();

    // Error should mention both field and type
    assert!(
        error_str.contains("age"),
        "Error should mention field 'age': {}",
        error_str
    );
    assert!(
        error_str.contains("type") || error_str.contains("mismatch"),
        "Error should indicate type issue: {}",
        error_str
    );
}

#[test]
fn test_expected_type_is_field_type_not_struct_type() {
    // Problem 2 fix: expected should be field type, not struct type
    let value = json!({
        "name": "Test",
        "age": "not_a_number"
    });

    let result = Person::from_value(&value);
    assert!(result.is_err());

    match result.unwrap_err() {
        CarveError::TypeMismatch { path, expected, found } => {
            assert_eq!(path.to_string(), "$.age");
            // Expected should mention "u32" (field type), NOT "Person" (struct type)
            assert!(
                expected.contains("u32"),
                "Expected should be field type 'u32', got: {}",
                expected
            );
            assert!(
                !expected.contains("Person"),
                "Expected should NOT be struct type 'Person', got: {}",
                expected
            );
            assert_eq!(found, "string");
        }
        other => panic!("Expected TypeMismatch, got: {:?}", other),
    }

    // Also test for other field types
    let value2 = json!({
        "name": 123, // Wrong type for String
        "age": 30
    });

    let result2 = Person::from_value(&value2);
    assert!(result2.is_err());

    match result2.unwrap_err() {
        CarveError::TypeMismatch { path, expected, found } => {
            assert_eq!(path.to_string(), "$.name");
            // Should mention String or alloc::string::String
            assert!(
                expected.contains("String") || expected.contains("string"),
                "Expected should be field type 'String', got: {}",
                expected
            );
            assert_eq!(found, "number");
        }
        other => panic!("Expected TypeMismatch, got: {:?}", other),
    }
}
