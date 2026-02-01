//! Integration tests for cross-module nested type references.

mod cross_module_tests;

use carve_state::{apply_patch, AccessorOps, CarveViewModel, WriterOps};
use cross_module_tests::models::{Company, Event, Person};
use cross_module_tests::types::{Address, ContactInfo, Metadata};
use serde_json::json;

// ============================================================================
// Basic cross-module nested reference
// ============================================================================

#[test]
fn test_cross_module_nested_reader() {
    let doc = json!({
        "name": "Alice",
        "age": 30,
        "address": {
            "street": "123 Main St",
            "city": "NYC",
            "zip": "10001"
        },
        "contact": {
            "email": "alice@example.com",
            "phone": "+1-555-0100"
        }
    });

    let reader = Person::read(&doc);

    assert_eq!(reader.name().unwrap(), "Alice");
    assert_eq!(reader.age().unwrap(), 30);

    // Access cross-module nested Address
    let addr = reader.address();
    assert_eq!(addr.street().unwrap(), "123 Main St");
    assert_eq!(addr.city().unwrap(), "NYC");
    assert_eq!(addr.zip().unwrap(), "10001");

    // Access cross-module nested ContactInfo
    let contact = reader.contact();
    assert_eq!(contact.email().unwrap(), "alice@example.com");
    assert_eq!(contact.phone().unwrap(), Some("+1-555-0100".to_string()));
}

#[test]
fn test_cross_module_nested_writer() {
    let mut writer = Person::write();

    writer.name("Bob");
    writer.age(25);

    {
        let mut addr = writer.address();
        addr.street("456 Oak Ave");
        addr.city("LA");
        addr.zip("90001");
    }

    {
        let mut contact = writer.contact();
        contact.email("bob@example.com");
        contact.phone(Some("+1-555-0200".to_string()));
    }

    let patch = writer.build();
    let result = apply_patch(&json!({}), &patch).unwrap();

    assert_eq!(result["name"], "Bob");
    assert_eq!(result["age"], 25);
    assert_eq!(result["address"]["street"], "456 Oak Ave");
    assert_eq!(result["address"]["city"], "LA");
    assert_eq!(result["address"]["zip"], "90001");
    assert_eq!(result["contact"]["email"], "bob@example.com");
    assert_eq!(result["contact"]["phone"], "+1-555-0200");
}

#[test]
fn test_cross_module_nested_from_value() {
    let value = json!({
        "name": "Charlie",
        "age": 35,
        "address": {
            "street": "789 Pine Rd",
            "city": "SF",
            "zip": "94102"
        },
        "contact": {
            "email": "charlie@example.com",
            "phone": null  // Explicitly null for Option<String>
        }
    });

    let person = Person::from_value(&value).unwrap();

    assert_eq!(person.name, "Charlie");
    assert_eq!(person.age, 35);
    assert_eq!(person.address.street, "789 Pine Rd");
    assert_eq!(person.address.city, "SF");
    assert_eq!(person.address.zip, "94102");
    assert_eq!(person.contact.email, "charlie@example.com");
    assert_eq!(person.contact.phone, None);
}

// ============================================================================
// Cross-module nested in Option and Vec
// ============================================================================

#[test]
fn test_cross_module_option_nested() {
    let doc = json!({
        "name": "Acme Corp",
        "headquarters": {
            "street": "100 Business Blvd",
            "city": "NYC",
            "zip": "10001"
        },
        "offices": [],
        "metadata": null
    });

    let reader = Company::read(&doc);

    assert_eq!(reader.name().unwrap(), "Acme Corp");

    // Option<Address> from another module
    let hq = reader.headquarters().unwrap();
    assert!(hq.is_some());
    let hq = hq.unwrap();
    assert_eq!(hq.street().unwrap(), "100 Business Blvd");
    assert_eq!(hq.city().unwrap(), "NYC");
}

#[test]
fn test_cross_module_vec_nested() {
    let mut writer = Company::write();

    writer.name("TechCorp");

    // Vec<Address> where Address is from another module
    writer.offices_push(Address {
        street: "Office 1".to_string(),
        city: "NYC".to_string(),
        zip: "10001".to_string(),
    });

    writer.offices_push(Address {
        street: "Office 2".to_string(),
        city: "SF".to_string(),
        zip: "94102".to_string(),
    });

    let patch = writer.build();
    let result = apply_patch(&json!({}), &patch).unwrap();

    assert_eq!(result["name"], "TechCorp");
    assert_eq!(result["offices"].as_array().unwrap().len(), 2);
    assert_eq!(result["offices"][0]["city"], "NYC");
    assert_eq!(result["offices"][1]["city"], "SF");
}

#[test]
fn test_cross_module_option_metadata() {
    let mut writer = Company::write();

    writer.name("StartupCo");

    // Option<Metadata> where Metadata is from another module
    {
        let mut meta = writer.metadata();
        meta.created_at("2024-01-01");
        meta.updated_at("2024-02-01");
    }

    let patch = writer.build();
    let result = apply_patch(&json!({}), &patch).unwrap();

    assert_eq!(result["name"], "StartupCo");
    assert_eq!(result["metadata"]["created_at"], "2024-01-01");
    assert_eq!(result["metadata"]["updated_at"], "2024-02-01");
}

// ============================================================================
// Multi-level cross-module nesting
// ============================================================================

#[test]
fn test_multi_level_cross_module_nesting() {
    // Event -> Person -> Address (3 levels, 2 module boundaries)
    let doc = json!({
        "event_name": "Conference 2024",
        "organizer": {
            "name": "Alice",
            "age": 30,
            "address": {
                "street": "123 Main",
                "city": "NYC",
                "zip": "10001"
            },
            "contact": {
                "email": "alice@conf.com",
                "phone": "+1-555-0100"
            }
        }
    });

    let reader = Event::read(&doc);

    assert_eq!(reader.event_name().unwrap(), "Conference 2024");

    // Navigate through nested cross-module types
    let organizer = reader.organizer();
    assert_eq!(organizer.name().unwrap(), "Alice");

    // Access deeply nested cross-module Address
    let addr = organizer.address();
    assert_eq!(addr.street().unwrap(), "123 Main");
    assert_eq!(addr.city().unwrap(), "NYC");
}

#[test]
fn test_multi_level_cross_module_writer() {
    let mut writer = Event::write();

    writer.event_name("Tech Summit 2024");

    {
        let mut organizer = writer.organizer();
        organizer.name("Bob");
        organizer.age(35);

        {
            let mut addr = organizer.address();
            addr.street("456 Oak");
            addr.city("SF");
            addr.zip("94102");
        }

        {
            let mut contact = organizer.contact();
            contact.email("bob@summit.com");
            contact.phone(None);
        }
    }

    let patch = writer.build();
    let result = apply_patch(&json!({}), &patch).unwrap();

    assert_eq!(result["event_name"], "Tech Summit 2024");
    assert_eq!(result["organizer"]["name"], "Bob");
    assert_eq!(result["organizer"]["age"], 35);
    assert_eq!(result["organizer"]["address"]["street"], "456 Oak");
    assert_eq!(result["organizer"]["address"]["city"], "SF");
    assert_eq!(result["organizer"]["contact"]["email"], "bob@summit.com");
    assert!(result["organizer"]["contact"]["phone"].is_null());
}

#[test]
fn test_multi_level_cross_module_accessor() {
    let doc = json!({
        "event_name": "Workshop",
        "organizer": {
            "name": "Charlie",
            "age": 28,
            "address": {
                "street": "789 Pine",
                "city": "Austin",
                "zip": "78701"
            },
            "contact": {
                "email": "charlie@workshop.com"
            }
        }
    });

    let accessor = Event::access(&doc);

    // Read through multiple levels
    let organizer_name = accessor.organizer().name().get().unwrap();
    assert_eq!(organizer_name, "Charlie");

    // Modify deeply nested cross-module field
    accessor.organizer().address().city().set("Houston".to_string());

    let patch = accessor.build();
    let result = apply_patch(&doc, &patch).unwrap();

    // Original fields unchanged
    assert_eq!(result["event_name"], "Workshop");
    assert_eq!(result["organizer"]["name"], "Charlie");
    assert_eq!(result["organizer"]["address"]["street"], "789 Pine");

    // Modified field updated
    assert_eq!(result["organizer"]["address"]["city"], "Houston");
}

// ============================================================================
// Error handling across modules
// ============================================================================

#[test]
fn test_cross_module_error_propagation() {
    let value = json!({
        "name": "Alice",
        "age": 30,
        "address": {
            "street": "123 Main",
            "city": "NYC"
            // "zip" is missing
        },
        "contact": {
            "email": "alice@example.com"
        }
    });

    let result = Person::from_value(&value);
    assert!(result.is_err());

    // Error should have full path through cross-module boundary
    let err = result.unwrap_err();
    let err_string = err.to_string();
    assert!(
        err_string.contains("address.zip") || err_string.contains("$.address.zip"),
        "Error should contain nested path: {}",
        err_string
    );
}

#[test]
fn test_cross_module_type_mismatch_error() {
    let value = json!({
        "event_name": "Test Event",
        "organizer": {
            "name": "Bob",
            "age": "not a number", // Type error
            "address": {
                "street": "123 Main",
                "city": "NYC",
                "zip": "10001"
            },
            "contact": {
                "email": "bob@example.com"
            }
        }
    });

    let result = Event::from_value(&value);
    assert!(result.is_err());

    let err = result.unwrap_err();
    let err_string = err.to_string();
    assert!(
        err_string.contains("organizer.age") || err_string.contains("$.organizer.age"),
        "Error should contain full nested path: {}",
        err_string
    );
}
