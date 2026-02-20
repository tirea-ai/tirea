//! Code generation for State derive macro.

mod state_ref;
mod utils;

use crate::field_kind::FieldKind;
use crate::parse::ViewModelInput;
use darling::FromDeriveInput;
use proc_macro2::TokenStream;
use syn::DeriveInput;

/// Main entry point for code generation.
pub fn expand(input: &DeriveInput) -> syn::Result<TokenStream> {
    let parsed = ViewModelInput::from_derive_input(input)
        .map_err(|e| syn::Error::new_spanned(input, e.to_string()))?;

    // Validate field attributes
    for field in parsed.fields() {
        // Check for flatten + rename conflict
        if field.flatten && field.rename.is_some() {
            return Err(syn::Error::new_spanned(
                field.ident(),
                "#[tirea(flatten)] and #[tirea(rename)] cannot be used together. \
                 Flattened fields are merged at the parent level, so rename has no effect.",
            ));
        }

        // Validate flatten fields
        if field.flatten {
            let kind = FieldKind::from_type(&field.ty, /* is_nested_attr = */ true);
            match kind {
                FieldKind::Nested => {
                    // Valid: flatten on a struct field
                }
                _ => {
                    return Err(syn::Error::new_spanned(
                        &field.ty,
                        "#[tirea(flatten)] currently only supports struct fields (non-Option/Vec/Map). \
                         The field must be a type that implements State.",
                    ));
                }
            }
        }
    }

    // Generate only the StateRef struct and State trait impl
    state_ref::generate(&parsed)
}
