//! Derive macro for carve-state `CarveViewModel` trait.
//!
//! This crate provides the `#[derive(CarveViewModel)]` macro that generates:
//! - `TReader<'a>`: Strongly-typed reader for accessing fields
//! - `TWriter`: Strongly-typed writer for modifying fields
//! - `impl CarveViewModel for T`: Trait implementation
//!
//! # Usage
//!
//! ```ignore
//! use carve_state::CarveViewModel;
//!
//! #[derive(CarveViewModel)]
//! struct User {
//!     name: String,
//!     age: u32,
//!     #[carve(nested)]
//!     profile: Profile,
//! }
//! ```

use proc_macro::TokenStream;
use syn::{parse_macro_input, DeriveInput};

mod codegen;
mod field_kind;
mod parse;

/// Derive the `CarveViewModel` trait for a struct.
///
/// This macro generates:
/// - A reader type `{StructName}Reader<'a>` with typed getter methods
/// - A writer type `{StructName}Writer` with typed setter methods
/// - `impl CarveViewModel for {StructName}`
/// - Convenience methods `{StructName}::read(&doc)` and `{StructName}::write()`
///
/// # Attributes
///
/// ## Field Attributes
///
/// - `#[carve(rename = "json_name")]`: Use a different name in JSON
/// - `#[carve(default = "expr")]`: Default value if field is missing
/// - `#[carve(skip)]`: Exclude from reader/writer
/// - `#[carve(nested)]`: Treat as nested CarveViewModel (auto-detected for structs)
/// - `#[carve(flatten)]`: Flatten nested struct fields
///
/// # Examples
///
/// ```ignore
/// #[derive(CarveViewModel)]
/// struct Counter {
///     value: i64,
///     #[carve(rename = "display_name")]
///     label: String,
/// }
///
/// // Generated reader usage
/// let doc = json!({"value": 42, "display_name": "My Counter"});
/// let reader = Counter::read(&doc);
/// assert_eq!(reader.value()?, 42);
/// assert_eq!(reader.label()?, "My Counter");
///
/// // Generated writer usage
/// let mut writer = Counter::write();
/// writer.value(100);
/// writer.label("Updated");
/// let patch = writer.build();
/// ```
#[proc_macro_derive(CarveViewModel, attributes(carve))]
pub fn derive_carve_view_model(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);

    match codegen::expand(&input) {
        Ok(tokens) => tokens.into(),
        Err(err) => err.to_compile_error().into(),
    }
}
