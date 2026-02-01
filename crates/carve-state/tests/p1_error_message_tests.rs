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
            assert!(expected.contains("Person")); // Type name included
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

    // First, Address::from_value should fail with path "city"
    let addr_value = value.get("address").unwrap();
    let addr_result = Address::from_value(addr_value);
    assert!(addr_result.is_err());

    match addr_result.unwrap_err() {
        CarveError::PathNotFound { path } => {
            assert_eq!(path.to_string(), "$.city");
        }
        other => panic!("Expected PathNotFound, got: {:?}", other),
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
