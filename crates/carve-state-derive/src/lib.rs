//! Derive macro for carve-state `State` trait.
//!
//! This crate provides the `#[derive(State)]` macro that generates:
//! - `{Name}Ref<'a>`: Typed state reference for reading and writing
//! - `impl State for {Name}`: Trait implementation
//!
//! # Usage
//!
//! ```ignore
//! use carve_state::State;
//! use carve_state_derive::State;
//!
//! #[derive(State)]
//! struct User {
//!     name: String,
//!     age: i64,
//!     #[carve(nested)]
//!     profile: Profile,
//! }
//! ```

use proc_macro::TokenStream;
use syn::{parse_macro_input, DeriveInput};

mod codegen;
mod field_kind;
mod parse;

/// Derive the `State` trait for a struct.
///
/// This macro generates:
/// - A reference type `{StructName}Ref<'a>` with typed getter and setter methods
/// - `impl State for {StructName}`
///
/// # Attributes
///
/// ## Field Attributes
///
/// - `#[carve(rename = "json_name")]`: Use a different name in JSON
/// - `#[carve(default = "expr")]`: Default value expression if field is missing
/// - `#[carve(skip)]`: Exclude from state ref (field must implement `Default`)
/// - `#[carve(nested)]`: Treat as nested State. **Required** for struct fields
///   that should have their own Ref type. Without this, the field is serialized as a whole value.
/// - `#[carve(flatten)]`: Flatten nested struct fields into parent
///
/// # Examples
///
/// ```ignore
/// use carve_state::{State, Context};
/// use carve_state_derive::State;
///
/// #[derive(State)]
/// struct Counter {
///     value: i64,
///     #[carve(rename = "display_name")]
///     label: String,
/// }
///
/// // Usage in a Context
/// let counter = ctx.state::<Counter>("counters.main");
///
/// // Read
/// let value = counter.value()?;
/// let label = counter.label()?;
///
/// // Write (automatically collected)
/// counter.set_value(100);
/// counter.set_label("Updated");
/// counter.increment_value(1);
/// ```
#[proc_macro_derive(State, attributes(carve))]
pub fn derive_state(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);

    match codegen::expand(&input) {
        Ok(tokens) => tokens.into(),
        Err(err) => err.to_compile_error().into(),
    }
}

/// Alias for `State` derive macro (for backwards compatibility).
#[proc_macro_derive(CarveViewModel, attributes(carve))]
pub fn derive_carve_view_model(input: TokenStream) -> TokenStream {
    derive_state(input)
}
