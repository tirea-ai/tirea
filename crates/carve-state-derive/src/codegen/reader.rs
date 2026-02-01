//! Reader code generation.

use crate::field_kind::FieldKind;
use crate::parse::{FieldInput, ViewModelInput};
use proc_macro2::TokenStream;
use quote::{format_ident, quote};

/// Generate the reader struct and impl.
pub fn generate(input: &ViewModelInput) -> syn::Result<TokenStream> {
    let struct_name = &input.ident;
    let reader_name = format_ident!("{}Reader", struct_name);
    let vis = &input.vis;

    let all_fields: Vec<_> = input.fields();
    let included_fields: Vec<_> = all_fields.iter().filter(|f| f.is_included()).copied().collect();

    let field_methods = generate_field_methods(&included_fields)?;
    let get_method = generate_get_method(struct_name, &all_fields)?;
    let has_methods = generate_has_methods(&included_fields)?;

    Ok(quote! {
        /// Strongly-typed reader for accessing fields.
        #vis struct #reader_name<'a> {
            doc: &'a ::serde_json::Value,
            base: ::carve_state::Path,
        }

        impl<'a> #reader_name<'a> {
            /// Create a new reader at the specified base path.
            #[doc(hidden)]
            pub fn new(doc: &'a ::serde_json::Value, base: ::carve_state::Path) -> Self {
                Self { doc, base }
            }

            #field_methods

            #get_method

            #has_methods
        }
    })
}

/// Generate getter methods for each field.
fn generate_field_methods(fields: &[&FieldInput]) -> syn::Result<TokenStream> {
    let mut methods = TokenStream::new();

    for field in fields {
        let method = generate_field_method(field)?;
        methods.extend(method);
    }

    Ok(methods)
}

/// Generate a getter method for a single field.
fn generate_field_method(field: &FieldInput) -> syn::Result<TokenStream> {
    let field_name = field.ident();
    let field_ty = &field.ty;
    let json_key = field.json_key();
    let kind = FieldKind::from_type(field_ty, field.nested);

    let method = match &kind {
        FieldKind::Nested => {
            // For nested types, use trait associated type (no string-based type name)
            quote! {
                /// Get a reader for the nested field.
                pub fn #field_name(&self) -> <#field_ty as ::carve_state::CarveViewModel>::Reader<'a> {
                    let mut path = self.base.clone();
                    path.push_key(#json_key);
                    <#field_ty as ::carve_state::CarveViewModel>::reader(self.doc, path)
                }
            }
        }
        FieldKind::Option(inner) if inner.is_nested() => {
            // Option<NestedType> - use trait associated type
            let inner_ty = extract_inner_type(field_ty);
            quote! {
                /// Get a reader for the optional nested field.
                ///
                /// Returns `Ok(None)` if the field is missing or null.
                /// Returns `Ok(Some(reader))` if the field exists and is not null.
                /// Returns `Err(TypeMismatch)` if the field exists but is not an object.
                pub fn #field_name(&self) -> ::carve_state::CarveResult<Option<<#inner_ty as ::carve_state::CarveViewModel>::Reader<'a>>> {
                    let mut path = self.base.clone();
                    path.push_key(#json_key);
                    match ::carve_state::get_at_path(self.doc, &path) {
                        None => Ok(None),
                        Some(value) if value.is_null() => Ok(None),
                        Some(value) => {
                            // Validate that the value is an object before creating the nested reader
                            if !value.is_object() {
                                return Err(::carve_state::CarveError::TypeMismatch {
                                    path,
                                    expected: "object",
                                    found: ::carve_state::value_type_name(value),
                                });
                            }
                            Ok(Some(<#inner_ty as ::carve_state::CarveViewModel>::reader(self.doc, path)))
                        }
                    }
                }
            }
        }
        FieldKind::Option(_) => {
            // P1-5: Explicitly distinguish missing, null, and present values
            quote! {
                /// Read the optional field value.
                ///
                /// Returns `Ok(None)` if the field is missing or explicitly null.
                /// Returns `Ok(Some(value))` if the field is present and not null.
                pub fn #field_name(&self) -> ::carve_state::CarveResult<#field_ty> {
                    let mut path = self.base.clone();
                    path.push_key(#json_key);

                    match ::carve_state::get_at_path(self.doc, &path) {
                        None => Ok(None), // Field missing
                        Some(value) if value.is_null() => Ok(None), // Field is null
                        Some(value) => {
                            // Field present, deserialize with better error
                            ::serde_json::from_value(value.clone()).map_err(|_| {
                                ::carve_state::CarveError::TypeMismatch {
                                    path,
                                    expected: std::any::type_name::<#field_ty>(),
                                    found: ::carve_state::value_type_name(value),
                                }
                            })
                        }
                    }
                }
            }
        }
        _ => {
            // For primitive types, deserialize the value
            if let Some(default) = &field.default {
                let expr: syn::Expr = syn::parse_str(default)
                    .map_err(|e| syn::Error::new_spanned(field_ty, format!("invalid default expression: {}", e)))?;
                // Strict semantics: missing → default, null or wrong type → error
                quote! {
                    /// Read the field value.
                    ///
                    /// Returns the default value if the field is missing.
                    /// Returns an error if the field is null or has the wrong type.
                    pub fn #field_name(&self) -> ::carve_state::CarveResult<#field_ty> {
                        let mut path = self.base.clone();
                        path.push_key(#json_key);

                        match ::carve_state::get_at_path(self.doc, &path) {
                            None => Ok(#expr), // Field missing: use default
                            Some(value) => {
                                // Field present (including null): deserialize or error
                                ::serde_json::from_value(value.clone()).map_err(|_| {
                                    ::carve_state::CarveError::TypeMismatch {
                                        path,
                                        expected: std::any::type_name::<#field_ty>(),
                                        found: ::carve_state::value_type_name(value),
                                    }
                                })
                            }
                        }
                    }
                }
            } else {
                // P1-4: Provide better error messages with path and type info
                quote! {
                    /// Read the field value.
                    pub fn #field_name(&self) -> ::carve_state::CarveResult<#field_ty> {
                        let mut path = self.base.clone();
                        path.push_key(#json_key);
                        let value = ::carve_state::get_at_path(self.doc, &path)
                            .ok_or_else(|| ::carve_state::CarveError::path_not_found(path.clone()))?;
                        ::serde_json::from_value(value.clone()).map_err(|_| {
                            ::carve_state::CarveError::TypeMismatch {
                                path,
                                expected: std::any::type_name::<#field_ty>(),
                                found: ::carve_state::value_type_name(value),
                            }
                        })
                    }
                }
            }
        }
    };

    Ok(method)
}

/// Generate the `get()` method that returns the entire struct.
///
/// This method reuses each field's getter to ensure consistent semantics
/// (default values, error paths, Option handling, etc.).
fn generate_get_method(struct_name: &syn::Ident, fields: &[&FieldInput]) -> syn::Result<TokenStream> {
    let field_reads: Vec<_> = fields
        .iter()
        .map(|f| {
            let name = f.ident();
            let kind = FieldKind::from_type(&f.ty, f.nested);

            if f.skip {
                // Skipped fields use Default::default()
                quote! {
                    #name: Default::default()
                }
            } else {
                match &kind {
                    FieldKind::Nested => {
                        // For Nested, reader.field() returns Reader, call .get() on it
                        quote! {
                            #name: self.#name().get()?
                        }
                    }
                    FieldKind::Option(inner) if inner.is_nested() => {
                        // For Option<Nested>, reader.field()? returns Result<Option<Reader>>
                        // Need to map the Option to call .get() on inner Reader
                        quote! {
                            #name: self.#name()?.map(|r| r.get()).transpose()?
                        }
                    }
                    _ => {
                        // For all other fields (primitive, Option, Vec, Map, etc.),
                        // call the field getter to reuse its logic (default, error paths, etc.)
                        quote! {
                            #name: self.#name()?
                        }
                    }
                }
            }
        })
        .collect();

    Ok(quote! {
        /// Get the entire struct value.
        pub fn get(&self) -> ::carve_state::CarveResult<#struct_name> {
            Ok(#struct_name {
                #(#field_reads),*
            })
        }
    })
}

/// Generate `has_*` methods for checking field existence.
fn generate_has_methods(fields: &[&FieldInput]) -> syn::Result<TokenStream> {
    let mut methods = TokenStream::new();

    for field in fields {
        let field_name = field.ident();
        let has_name = format_ident!("has_{}", field_name);
        let json_key = field.json_key();

        methods.extend(quote! {
            /// Check if the field exists.
            pub fn #has_name(&self) -> bool {
                let mut path = self.base.clone();
                path.push_key(#json_key);
                ::carve_state::get_at_path(self.doc, &path).is_some()
            }
        });
    }

    Ok(methods)
}

/// Extract the type name from a Type.
#[allow(dead_code)]
fn get_type_name(ty: &syn::Type) -> String {
    match ty {
        syn::Type::Path(type_path) => {
            if let Some(segment) = type_path.path.segments.last() {
                segment.ident.to_string()
            } else {
                "Unknown".to_string()
            }
        }
        _ => "Unknown".to_string(),
    }
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
