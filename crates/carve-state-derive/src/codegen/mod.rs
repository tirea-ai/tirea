//! Code generation for CarveViewModel derive macro.

mod accessor;
mod reader;
mod utils;
mod view_model;
mod writer;

use crate::field_kind::FieldKind;
use crate::parse::ViewModelInput;
use darling::FromDeriveInput;
use proc_macro2::TokenStream;
use syn::DeriveInput;

/// Main entry point for code generation.
pub fn expand(input: &DeriveInput) -> syn::Result<TokenStream> {
    let parsed = ViewModelInput::from_derive_input(input)
        .map_err(|e| syn::Error::new_spanned(input, e.to_string()))?;

    // Validate flatten fields
    for field in parsed.fields() {
        if field.flatten {
            // flatten is treated as implicitly nested
            // Validate that flatten is only used on struct fields (not Option/Vec/Map)
            let kind = FieldKind::from_type(&field.ty, /* is_nested_attr = */ true);
            match kind {
                FieldKind::Nested => {
                    // Valid: flatten on a struct field
                }
                _ => {
                    return Err(syn::Error::new_spanned(
                        &field.ty,
                        "#[carve(flatten)] currently only supports struct fields (non-Option/Vec/Map). \
                         The field must be a type that implements CarveViewModel."
                    ));
                }
            }
        }
    }

    let reader_tokens = reader::generate(&parsed)?;
    let writer_tokens = writer::generate(&parsed)?;
    let accessor_tokens = accessor::generate(&parsed)?;
    let view_model_tokens = view_model::generate(&parsed)?;

    Ok(quote::quote! {
        #reader_tokens
        #writer_tokens
        #accessor_tokens
        #view_model_tokens
    })
}
