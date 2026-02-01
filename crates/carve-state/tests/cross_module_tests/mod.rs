//! Integration tests for cross-module nested type references.
//!
//! This test suite verifies that CarveViewModel works correctly when:
//! - Nested types are defined in different modules
//! - Types are imported via `use` statements
//! - Multiple levels of nesting across modules exist

pub mod models;
pub mod types;
