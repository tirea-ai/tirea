//! CarveViewModel trait implementation code generation.

use crate::field_kind::FieldKind;
use crate::parse::{FieldInput, ViewModelInput};
use proc_macro2::TokenStream;
use quote::{format_ident, quote};

/// Generate the CarveViewModel trait implementation.
pub fn generate(input: &ViewModelInput) -> syn::Result<TokenStream> {
    let struct_name = &input.ident;
    let reader_name = format_ident!("{}Reader", struct_name);
    let writer_name = format_ident!("{}Writer", struct_name);
    let accessor_name = format_ident!("{}Accessor", struct_name);
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
            type Accessor<'a> = #accessor_name<'a> where Self: 'a;

            fn reader(doc: &::serde_json::Value, base: ::carve_state::Path) -> Self::Reader<'_> {
                #reader_name::new(doc, base)
            }

            fn writer(base: ::carve_state::Path) -> Self::Writer {
                #writer_name::new(base)
            }

            fn accessor(doc: &::serde_json::Value, base: ::carve_state::Path) -> Self::Accessor<'_> {
                #accessor_name::new(doc, base)
            }

            fn from_value(value: &::serde_json::Value) -> ::carve_state::CarveResult<Self> {
                #from_value_body
            }

            fn to_value(&self) -> ::serde_json::Value {
                #to_value_body
            }
        }

        /// Convenience methods for creating readers, writers, and accessors.
        impl #impl_generics #struct_name #ty_generics #where_clause {
            /// Create a reader at the document root.
            #vis fn read(doc: &::serde_json::Value) -> #reader_name<'_> {
                #reader_name::new(doc, ::carve_state::Path::root())
            }

            /// Create a writer at the document root.
            #vis fn write() -> #writer_name {
                #writer_name::new(::carve_state::Path::root())
            }

            /// Create an accessor at the document root.
            ///
            /// The accessor combines read and write capabilities with
            /// field proxy types for operator support.
            #vis fn access(doc: &::serde_json::Value) -> #accessor_name<'_> {
                #accessor_name::new(doc, ::carve_state::Path::root())
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
                // P1-4 + P1-3: Better error messages with path information
                // and recursive CarveViewModel::from_value for nested types
                let field_ty = &f.ty;
                let kind = FieldKind::from_type(field_ty, f.nested);

                match &kind {
                    FieldKind::Nested => {
                        // Nested type: use CarveViewModel::from_value recursively
                        quote! {
                            #name: {
                                let mut field_path = ::carve_state::Path::root();
                                field_path.push_key(#json_key);

                                match value.get(#json_key) {
                                    None => {
                                        return Err(::carve_state::CarveError::PathNotFound {
                                            path: field_path,
                                        });
                                    }
                                    Some(field_value) => {
                                        <#field_ty as ::carve_state::CarveViewModel>::from_value(field_value)
                                            .map_err(|e| e.with_prefix(&field_path))?
                                    }
                                }
                            }
                        }
                    }
                    FieldKind::Option(inner) if inner.is_nested() => {
                        // Option<Nested>: handle null/missing, then recurse
                        let inner_ty = extract_inner_type(field_ty);
                        quote! {
                            #name: {
                                let mut field_path = ::carve_state::Path::root();
                                field_path.push_key(#json_key);

                                match value.get(#json_key) {
                                    None => None,
                                    Some(field_value) if field_value.is_null() => None,
                                    Some(field_value) => {
                                        Some(
                                            <#inner_ty as ::carve_state::CarveViewModel>::from_value(field_value)
                                                .map_err(|e| e.with_prefix(&field_path))?
                                        )
                                    }
                                }
                            }
                        }
                    }
                    _ => {
                        // Non-nested: use serde_json::from_value
                        // Fix problem 2: use field type, not Self
                        quote! {
                            #name: {
                                let mut field_path = ::carve_state::Path::root();
                                field_path.push_key(#json_key);

                                match value.get(#json_key) {
                                    None => {
                                        return Err(::carve_state::CarveError::PathNotFound {
                                            path: field_path,
                                        });
                                    }
                                    Some(field_value) => {
                                        ::serde_json::from_value(field_value.clone()).map_err(|_| {
                                            ::carve_state::CarveError::TypeMismatch {
                                                path: field_path,
                                                expected: std::any::type_name::<#field_ty>(),
                                                found: ::carve_state::value_type_name(field_value),
                                            }
                                        })?
                                    }
                                }
                            }
                        }
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

/// Extract the inner type from Option<T> or Vec<T>.
fn extract_inner_type(ty: &syn::Type) -> syn::Type {
    if let syn::Type::Path(type_path) = ty {
        if let Some(segment) = type_path.path.segments.last() {
            if let syn::PathArguments::AngleBracketed(ab) = &segment.arguments {
                if let Some(syn::GenericArgument::Type(inner)) = ab.args.first() {
                    return inner.clone();
                }
            }
        }
    }
    ty.clone()
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
