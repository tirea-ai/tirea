//! CarveViewModel trait implementation code generation.

use crate::parse::{FieldInput, ViewModelInput};
use proc_macro2::TokenStream;
use quote::{format_ident, quote};

/// Generate the CarveViewModel trait implementation.
pub fn generate(input: &ViewModelInput) -> syn::Result<TokenStream> {
    let struct_name = &input.ident;
    let reader_name = format_ident!("{}Reader", struct_name);
    let writer_name = format_ident!("{}Writer", struct_name);
    let vis = &input.vis;
    let (impl_generics, ty_generics, where_clause) = input.generics.split_for_impl();

    let all_fields: Vec<_> = input.fields();
    let included_fields: Vec<_> = all_fields.iter().filter(|f| f.is_included()).copied().collect();

    let from_value_body = generate_from_value(&all_fields)?;
    let to_value_body = generate_to_value(&included_fields)?;

    Ok(quote! {
        impl #impl_generics ::carve_state::CarveViewModel for #struct_name #ty_generics #where_clause {
            type Reader<'a> = #reader_name<'a> where Self: 'a;
            type Writer = #writer_name;

            fn reader(doc: &::serde_json::Value, base: ::carve_state::Path) -> Self::Reader<'_> {
                #reader_name::new(doc, base)
            }

            fn writer(base: ::carve_state::Path) -> Self::Writer {
                #writer_name::new(base)
            }

            fn from_value(value: &::serde_json::Value) -> ::carve_state::CarveResult<Self> {
                #from_value_body
            }

            fn to_value(&self) -> ::serde_json::Value {
                #to_value_body
            }
        }

        /// Convenience methods for creating readers and writers.
        impl #impl_generics #struct_name #ty_generics #where_clause {
            /// Create a reader at the document root.
            #vis fn read(doc: &::serde_json::Value) -> #reader_name<'_> {
                #reader_name::new(doc, ::carve_state::Path::root())
            }

            /// Create a writer at the document root.
            #vis fn write() -> #writer_name {
                #writer_name::new(::carve_state::Path::root())
            }
        }
    })
}

/// Generate the from_value implementation.
fn generate_from_value(fields: &[&FieldInput]) -> syn::Result<TokenStream> {
    let field_extractions: Vec<_> = fields
        .iter()
        .map(|f| {
            let name = f.ident();
            let json_key = f.json_key();

            if f.skip {
                // Skipped fields use Default::default()
                quote! {
                    #name: Default::default()
                }
            } else if let Some(default) = &f.default {
                let default_expr: syn::Expr = syn::parse_str(default)
                    .unwrap_or_else(|_| syn::parse_quote!(Default::default()));
                quote! {
                    #name: value.get(#json_key)
                        .and_then(|v| ::serde_json::from_value(v.clone()).ok())
                        .unwrap_or_else(|| #default_expr)
                }
            } else {
                quote! {
                    #name: {
                        let field_value = value.get(#json_key)
                            .cloned()
                            .unwrap_or(::serde_json::Value::Null);
                        ::serde_json::from_value(field_value)?
                    }
                }
            }
        })
        .collect();

    Ok(quote! {
        // Verify the value is an object
        let _ = value.as_object()
            .ok_or_else(|| ::carve_state::CarveError::type_mismatch(
                ::carve_state::Path::root(),
                "object",
                ::carve_state::value_type_name(value),
            ))?;

        Ok(Self {
            #(#field_extractions),*
        })
    })
}

/// Generate the to_value implementation.
fn generate_to_value(fields: &[&FieldInput]) -> syn::Result<TokenStream> {
    let field_serializations: Vec<_> = fields
        .iter()
        .map(|f| {
            let name = f.ident();
            let json_key = f.json_key();

            quote! {
                map.insert(
                    #json_key.to_string(),
                    ::serde_json::to_value(&self.#name).unwrap_or(::serde_json::Value::Null)
                );
            }
        })
        .collect();

    Ok(quote! {
        let mut map = ::serde_json::Map::new();
        #(#field_serializations)*
        ::serde_json::Value::Object(map)
    })
}
