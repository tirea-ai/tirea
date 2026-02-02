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
        .map(|f| -> syn::Result<proc_macro2::TokenStream> {
            let name = f.ident();
            let json_key = f.json_key();

            if f.skip {
                // Skipped fields use Default::default()
                Ok(quote! {
                    #name: Default::default()
                })
            } else if let Some(default) = &f.default {
                let default_expr: syn::Expr = syn::parse_str(default)
                    .map_err(|e| syn::Error::new_spanned(
                        &f.ty,
                        format!("invalid default expression '{}': {}", default, e)
                    ))?;
                let field_ty = &f.ty;
                // Problem 4 fix: missing → default, present but wrong type → error
                Ok(quote! {
                    #name: {
                        let mut field_path = ::carve_state::Path::root();
                        field_path.push_key(#json_key);

                        match value.get(#json_key) {
                            None => #default_expr, // Field missing: use default
                            Some(field_value) => {
                                // Field present: deserialize or error
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
                })
            } else {
                // P1-4 + P1-3: Better error messages with path information
                // and recursive CarveViewModel::from_value for nested types
                let field_ty = &f.ty;
                // flatten is implicitly nested
                let kind = FieldKind::from_type(field_ty, f.flatten || f.nested);

                match &kind {
                    FieldKind::Nested => {
                        if f.flatten {
                            // Flatten: deserialize from the entire value, not from value.get(key)
                            // Error paths don't need prefix since the fields are at root level
                            Ok(quote! {
                                #name: <#field_ty as ::carve_state::CarveViewModel>::from_value(value)?
                            })
                        } else {
                            // Normal nested: use CarveViewModel::from_value recursively
                            Ok(quote! {
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
                            })
                        }
                    }
                    FieldKind::Option(inner) if inner.is_nested() => {
                        // Option<Nested>: handle null/missing, then recurse
                        let inner_ty = extract_inner_type(field_ty);
                        Ok(quote! {
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
                        })
                    }
                    FieldKind::Vec(inner) if inner.is_nested() => {
                        // Vec<Nested>: iterate array elements with indexed paths
                        let inner_ty = extract_inner_type(field_ty);
                        Ok(quote! {
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
                                        let arr = field_value.as_array()
                                            .ok_or_else(|| ::carve_state::CarveError::TypeMismatch {
                                                path: field_path.clone(),
                                                expected: "array",
                                                found: ::carve_state::value_type_name(field_value),
                                            })?;

                                        let mut result = Vec::with_capacity(arr.len());
                                        for (i, elem) in arr.iter().enumerate() {
                                            let mut elem_path = field_path.clone();
                                            elem_path.push_index(i);

                                            let item = <#inner_ty as ::carve_state::CarveViewModel>::from_value(elem)
                                                .map_err(|e| e.with_prefix(&elem_path))?;
                                            result.push(item);
                                        }
                                        result
                                    }
                                }
                            }
                        })
                    }
                    FieldKind::Map { value: value_kind, .. } if value_kind.is_nested() => {
                        // Map<String, Nested>: iterate object entries with keyed paths
                        let inner_ty = extract_second_generic_arg(field_ty);
                        Ok(quote! {
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
                                        let obj = field_value.as_object()
                                            .ok_or_else(|| ::carve_state::CarveError::TypeMismatch {
                                                path: field_path.clone(),
                                                expected: "object",
                                                found: ::carve_state::value_type_name(field_value),
                                            })?;

                                        let mut result = std::collections::HashMap::new();
                                        for (key, val) in obj.iter() {
                                            let mut entry_path = field_path.clone();
                                            entry_path.push_key(key);

                                            let item = <#inner_ty as ::carve_state::CarveViewModel>::from_value(val)
                                                .map_err(|e| e.with_prefix(&entry_path))?;
                                            result.insert(key.clone(), item);
                                        }
                                        result
                                    }
                                }
                            }
                        })
                    }
                    _ => {
                        // Non-nested: use serde_json::from_value
                        // Fix problem 2: use field type, not Self
                        Ok(quote! {
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
                        })
                    }
                }
            }
        })
        .collect::<syn::Result<Vec<_>>>()?;

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

/// Extract the second generic argument from Map<K, V>.
fn extract_second_generic_arg(ty: &syn::Type) -> syn::Type {
    if let syn::Type::Path(type_path) = ty {
        if let Some(segment) = type_path.path.segments.last() {
            if let syn::PathArguments::AngleBracketed(ab) = &segment.arguments {
                if let Some(syn::GenericArgument::Type(second)) = ab.args.iter().nth(1) {
                    return second.clone();
                }
            }
        }
    }
    ty.clone()
}

/// Generate the to_value implementation.
fn generate_to_value(fields: &[&FieldInput]) -> syn::Result<TokenStream> {
    // Separate flatten and normal fields
    let flatten_fields: Vec<_> = fields.iter().filter(|f| f.flatten).copied().collect();
    let normal_fields: Vec<_> = fields.iter().filter(|f| !f.flatten).copied().collect();

    // Generate flatten field serializations (merge object entries)
    let flatten_serializations: Vec<_> = flatten_fields
        .iter()
        .map(|f| {
            let name = f.ident();
            quote! {
                // Flatten: merge the nested struct's fields into the parent object
                let flattened_value = ::carve_state::CarveViewModel::to_value(&self.#name);
                if let ::serde_json::Value::Object(obj) = flattened_value {
                    for (k, v) in obj {
                        map.insert(k, v);
                    }
                }
            }
        })
        .collect();

    // Generate normal field serializations
    let normal_serializations: Vec<_> = normal_fields
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

    // Insert flatten fields first, then normal fields
    // This ensures normal fields override flatten fields in case of key conflicts
    Ok(quote! {
        let mut map = ::serde_json::Map::new();
        #(#flatten_serializations)*
        #(#normal_serializations)*
        ::serde_json::Value::Object(map)
    })
}
