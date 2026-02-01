//! Code generation for CarveViewModel derive macro.

mod accessor;
mod reader;
mod view_model;
mod writer;

use crate::parse::ViewModelInput;
use darling::FromDeriveInput;
use proc_macro2::TokenStream;
use syn::DeriveInput;

/// Main entry point for code generation.
pub fn expand(input: &DeriveInput) -> syn::Result<TokenStream> {
    let parsed = ViewModelInput::from_derive_input(input)
        .map_err(|e| syn::Error::new_spanned(input, e.to_string()))?;

    // P1-7: Reject flatten attribute (not yet implemented)
    for field in parsed.fields() {
        if field.flatten {
            return Err(syn::Error::new_spanned(
                &field.ty,
                "#[carve(flatten)] is not yet implemented. \
                 Please use nested types with explicit paths instead."
            ));
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
