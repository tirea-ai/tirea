//! Code generation for CarveViewModel derive macro.

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

    let reader_tokens = reader::generate(&parsed)?;
    let writer_tokens = writer::generate(&parsed)?;
    let view_model_tokens = view_model::generate(&parsed)?;

    Ok(quote::quote! {
        #reader_tokens
        #writer_tokens
        #view_model_tokens
    })
}
