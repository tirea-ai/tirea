//! Accessor code generation.
//!
//! Generates a unified Accessor type that combines Reader and Writer functionality
//! with field-like access using proxy types.

use crate::field_kind::FieldKind;
use crate::parse::{FieldInput, ViewModelInput};
use proc_macro2::TokenStream;
use quote::{format_ident, quote};

/// Generate the accessor struct and impl.
pub fn generate(input: &ViewModelInput) -> syn::Result<TokenStream> {
    let struct_name = &input.ident;
    let accessor_name = format_ident!("{}Accessor", struct_name);
    let vis = &input.vis;

    let fields: Vec<_> = input
        .fields()
        .into_iter()
        .filter(|f| f.is_included())
        .collect();

    let field_accessors = generate_field_accessors(&fields)?;
    let accessor_ops_impl = generate_accessor_ops_impl(&accessor_name)?;
    let nested_accessor_guards = generate_nested_accessor_guards(struct_name, &fields)?;

    Ok(quote! {
        /// Unified accessor for reading and writing fields.
        ///
        /// This type combines the functionality of Reader and Writer,
        /// allowing both read and write operations through a single interface.
        #vis struct #accessor_name<'a> {
            doc: &'a ::serde_json::Value,
            base: ::carve_state::Path,
            // Private: guards can access because they're in the same generated code scope
            ops: ::std::cell::RefCell<Vec<::carve_state::Op>>,
        }

        impl<'a> #accessor_name<'a> {
            /// Create a new accessor at the specified base path.
            #[doc(hidden)]
            pub fn new(doc: &'a ::serde_json::Value, base: ::carve_state::Path) -> Self {
                Self {
                    doc,
                    base,
                    ops: ::std::cell::RefCell::new(Vec::new()),
                }
            }

            /// Internal accessor for ops (used by guards).
            ///
            /// This is public only for generated code access and should not be used directly.
            #[doc(hidden)]
            #[inline]
            pub fn ops_cell(&self) -> &::std::cell::RefCell<Vec<::carve_state::Op>> {
                &self.ops
            }

            #field_accessors
        }

        #accessor_ops_impl

        #nested_accessor_guards
    })
}

/// Generate field accessor methods for each field.
fn generate_field_accessors(fields: &[&FieldInput]) -> syn::Result<TokenStream> {
    let mut methods = TokenStream::new();

    for field in fields {
        let accessor = generate_field_accessor(field)?;
        methods.extend(accessor);
    }

    Ok(methods)
}

/// Generate accessor method for a single field.
fn generate_field_accessor(field: &FieldInput) -> syn::Result<TokenStream> {
    let field_name = field.ident();
    let field_ty = &field.ty;
    let json_key = field.json_key();
    let kind = FieldKind::from_type(field_ty, field.nested);

    let methods = match &kind {
        FieldKind::Nested => {
            // P0-3: Guard name includes field name to avoid conflicts
            // Convert to PascalCase
            let pascal = to_pascal_case(&field_name.to_string());
            let guard_name = format_ident!("{}AccessorGuard", pascal);
            quote! {
                /// Get an accessor for the nested field.
                ///
                /// The nested accessor's operations are automatically merged
                /// into the parent when the guard is dropped.
                pub fn #field_name(&self) -> #guard_name<'_> {
                    let mut path = self.base.clone();
                    path.push_key(#json_key);
                    #guard_name {
                        parent_ops: &self.ops,
                        // P0-2: Use trait associated type
                        accessor: <#field_ty as ::carve_state::CarveViewModel>::accessor(self.doc, path),
                    }
                }
            }
        }
        FieldKind::Option(inner) if inner.is_nested() => {
            // Option<NestedType> - P0-2: Use trait associated type
            let inner_ty = extract_inner_type(field_ty);
            let set_name = format_ident!("set_{}", field_name);
            let set_none_name = format_ident!("set_{}_none", field_name);
            let delete_name = format_ident!("delete_{}", field_name);

            quote! {
                /// Get an accessor for the optional nested field.
                ///
                /// Returns `Ok(None)` if the field is missing or null.
                pub fn #field_name(&self) -> ::carve_state::CarveResult<Option<<#inner_ty as ::carve_state::CarveViewModel>::Accessor<'a>>> {
                    let mut path = self.base.clone();
                    path.push_key(#json_key);
                    match ::carve_state::get_at_path(self.doc, &path) {
                        None => Ok(None),
                        Some(value) if value.is_null() => Ok(None),
                        Some(_) => Ok(Some(<#inner_ty as ::carve_state::CarveViewModel>::accessor(self.doc, path))),
                    }
                }

                /// Set the optional nested field value.
                pub fn #set_name(&self, value: #inner_ty) {
                    let mut path = self.base.clone();
                    path.push_key(#json_key);
                    self.ops.borrow_mut().push(::carve_state::Op::Set {
                        path,
                        value: ::serde_json::to_value(&value).unwrap_or(::serde_json::Value::Null),
                    });
                }

                /// Set the optional field to null (None).
                pub fn #set_none_name(&self) {
                    let mut path = self.base.clone();
                    path.push_key(#json_key);
                    self.ops.borrow_mut().push(::carve_state::Op::Set {
                        path,
                        value: ::serde_json::Value::Null,
                    });
                }

                /// Delete the optional nested field entirely.
                pub fn #delete_name(&self) {
                    let mut path = self.base.clone();
                    path.push_key(#json_key);
                    self.ops.borrow_mut().push(::carve_state::Op::Delete { path });
                }
            }
        }
        FieldKind::Option(_) => {
            // Option<T> for primitive types
            let inner_ty = extract_inner_type(field_ty);
            let set_name = format_ident!("set_{}", field_name);
            let set_none_name = format_ident!("set_{}_none", field_name);
            let delete_name = format_ident!("delete_{}", field_name);

            quote! {
                /// Get the optional field as an OptionField proxy.
                pub fn #field_name(&self) -> ::carve_state::OptionField<'_, #inner_ty> {
                    let mut path = self.base.clone();
                    path.push_key(#json_key);
                    ::carve_state::OptionField::new(self.doc, path, &self.ops)
                }

                /// Set the optional field value.
                pub fn #set_name(&self, value: #inner_ty) {
                    let mut path = self.base.clone();
                    path.push_key(#json_key);
                    self.ops.borrow_mut().push(::carve_state::Op::Set {
                        path,
                        value: ::serde_json::to_value(&value).unwrap_or(::serde_json::Value::Null),
                    });
                }

                /// Set the optional field to null (None).
                pub fn #set_none_name(&self) {
                    let mut path = self.base.clone();
                    path.push_key(#json_key);
                    self.ops.borrow_mut().push(::carve_state::Op::Set {
                        path,
                        value: ::serde_json::Value::Null,
                    });
                }

                /// Delete the optional field entirely.
                pub fn #delete_name(&self) {
                    let mut path = self.base.clone();
                    path.push_key(#json_key);
                    self.ops.borrow_mut().push(::carve_state::Op::Delete { path });
                }
            }
        }
        FieldKind::Vec(inner) => {
            let inner_ty = extract_inner_type(field_ty);
            let set_name = format_ident!("set_{}", field_name);
            let push_name = format_ident!("{}_push", field_name);
            let delete_name = format_ident!("delete_{}", field_name);

            if inner.is_nested() {
                // Vec<Nested>
                quote! {
                    /// Get the vec field as a VecField proxy.
                    pub fn #field_name(&self) -> ::carve_state::VecField<'_, #inner_ty> {
                        let mut path = self.base.clone();
                        path.push_key(#json_key);
                        ::carve_state::VecField::new(self.doc, path, &self.ops)
                    }

                    /// Set the entire vec.
                    pub fn #set_name(&self, value: #field_ty) {
                        let mut path = self.base.clone();
                        path.push_key(#json_key);
                        self.ops.borrow_mut().push(::carve_state::Op::Set {
                            path,
                            value: ::serde_json::to_value(&value).unwrap_or(::serde_json::Value::Null),
                        });
                    }

                    /// Push an item to the vec.
                    pub fn #push_name(&self, item: #inner_ty) {
                        let mut path = self.base.clone();
                        path.push_key(#json_key);
                        self.ops.borrow_mut().push(::carve_state::Op::Append {
                            path,
                            value: ::serde_json::to_value(&item).unwrap_or(::serde_json::Value::Null),
                        });
                    }

                    /// Delete the entire vec field.
                    pub fn #delete_name(&self) {
                        let mut path = self.base.clone();
                        path.push_key(#json_key);
                        self.ops.borrow_mut().push(::carve_state::Op::Delete { path });
                    }
                }
            } else {
                // Vec<Primitive>
                quote! {
                    /// Get the vec field as a VecField proxy.
                    pub fn #field_name(&self) -> ::carve_state::VecField<'_, #inner_ty> {
                        let mut path = self.base.clone();
                        path.push_key(#json_key);
                        ::carve_state::VecField::new(self.doc, path, &self.ops)
                    }

                    /// Set the entire vec.
                    pub fn #set_name(&self, value: #field_ty) {
                        let mut path = self.base.clone();
                        path.push_key(#json_key);
                        self.ops.borrow_mut().push(::carve_state::Op::Set {
                            path,
                            value: ::serde_json::to_value(&value).unwrap_or(::serde_json::Value::Null),
                        });
                    }

                    /// Push an item to the vec.
                    pub fn #push_name(&self, item: impl Into<#inner_ty>) {
                        let mut path = self.base.clone();
                        path.push_key(#json_key);
                        let v: #inner_ty = item.into();
                        self.ops.borrow_mut().push(::carve_state::Op::Append {
                            path,
                            value: ::serde_json::to_value(&v).unwrap_or(::serde_json::Value::Null),
                        });
                    }

                    /// Delete the entire vec field.
                    pub fn #delete_name(&self) {
                        let mut path = self.base.clone();
                        path.push_key(#json_key);
                        self.ops.borrow_mut().push(::carve_state::Op::Delete { path });
                    }
                }
            }
        }
        FieldKind::Map { .. } => {
            let (key_ty, value_ty) = extract_map_types(field_ty);
            let key_type_name = get_type_name(&key_ty);
            let set_name = format_ident!("set_{}", field_name);
            let delete_name = format_ident!("delete_{}", field_name);

            // Only generate insert method for String keys
            let insert_method = if key_type_name == "String" {
                let insert_name = format_ident!("{}_insert", field_name);
                let remove_name = format_ident!("{}_remove", field_name);
                quote! {
                    /// Insert a key-value pair into the map.
                    pub fn #insert_name(&self, key: impl Into<String>, value: impl Into<#value_ty>) {
                        let mut path = self.base.clone();
                        path.push_key(#json_key);
                        let k: String = key.into();
                        path.push_key(k);
                        let v: #value_ty = value.into();
                        self.ops.borrow_mut().push(::carve_state::Op::Set {
                            path,
                            value: ::serde_json::to_value(&v).unwrap_or(::serde_json::Value::Null),
                        });
                    }

                    /// Remove a key from the map.
                    pub fn #remove_name(&self, key: &str) {
                        let mut path = self.base.clone();
                        path.push_key(#json_key);
                        path.push_key(key);
                        self.ops.borrow_mut().push(::carve_state::Op::Delete { path });
                    }
                }
            } else {
                quote! {}
            };

            quote! {
                /// Get the map field as a MapField proxy.
                pub fn #field_name(&self) -> ::carve_state::MapField<'_, #key_ty, #value_ty> {
                    let mut path = self.base.clone();
                    path.push_key(#json_key);
                    ::carve_state::MapField::new(self.doc, path, &self.ops)
                }

                /// Set the entire map.
                pub fn #set_name(&self, value: #field_ty) {
                    let mut path = self.base.clone();
                    path.push_key(#json_key);
                    self.ops.borrow_mut().push(::carve_state::Op::Set {
                        path,
                        value: ::serde_json::to_value(&value).unwrap_or(::serde_json::Value::Null),
                    });
                }

                #insert_method

                /// Delete the entire map field.
                pub fn #delete_name(&self) {
                    let mut path = self.base.clone();
                    path.push_key(#json_key);
                    self.ops.borrow_mut().push(::carve_state::Op::Delete { path });
                }
            }
        }
        FieldKind::Primitive => {
            let type_name = get_type_name(field_ty);
            let set_name = format_ident!("set_{}", field_name);
            let delete_name = format_ident!("delete_{}", field_name);

            // Check for numeric types to return ScalarField with operator support
            let is_numeric = matches!(
                type_name.as_str(),
                "i32" | "i64" | "f32" | "f64" | "u32" | "u64" | "i8" | "i16" | "u8" | "u16"
            );

            if is_numeric || type_name == "String" || type_name == "bool" {
                // Return ScalarField proxy for operator support
                let set_method = if type_name == "String" {
                    quote! {
                        /// Set the field value.
                        pub fn #set_name(&self, value: impl Into<String>) {
                            let mut path = self.base.clone();
                            path.push_key(#json_key);
                            let v: String = value.into();
                            self.ops.borrow_mut().push(::carve_state::Op::Set {
                                path,
                                value: ::serde_json::Value::String(v),
                            });
                        }
                    }
                } else {
                    quote! {
                        /// Set the field value.
                        pub fn #set_name(&self, value: #field_ty) {
                            let mut path = self.base.clone();
                            path.push_key(#json_key);
                            self.ops.borrow_mut().push(::carve_state::Op::Set {
                                path,
                                value: ::serde_json::to_value(&value).unwrap_or(::serde_json::Value::Null),
                            });
                        }
                    }
                };

                quote! {
                    /// Get the field as a ScalarField proxy.
                    ///
                    /// This provides both read access and operator support (+=, -=, etc).
                    pub fn #field_name(&self) -> ::carve_state::ScalarField<'_, #field_ty> {
                        let mut path = self.base.clone();
                        path.push_key(#json_key);
                        ::carve_state::ScalarField::new(self.doc, path, &self.ops)
                    }

                    #set_method

                    /// Delete the field.
                    pub fn #delete_name(&self) {
                        let mut path = self.base.clone();
                        path.push_key(#json_key);
                        self.ops.borrow_mut().push(::carve_state::Op::Delete { path });
                    }
                }
            } else {
                // Generic type - return ScalarField
                quote! {
                    /// Get the field as a ScalarField proxy.
                    pub fn #field_name(&self) -> ::carve_state::ScalarField<'_, #field_ty> {
                        let mut path = self.base.clone();
                        path.push_key(#json_key);
                        ::carve_state::ScalarField::new(self.doc, path, &self.ops)
                    }

                    /// Set the field value.
                    pub fn #set_name(&self, value: #field_ty) {
                        let mut path = self.base.clone();
                        path.push_key(#json_key);
                        self.ops.borrow_mut().push(::carve_state::Op::Set {
                            path,
                            value: ::serde_json::to_value(&value).unwrap_or(::serde_json::Value::Null),
                        });
                    }

                    /// Delete the field.
                    pub fn #delete_name(&self) {
                        let mut path = self.base.clone();
                        path.push_key(#json_key);
                        self.ops.borrow_mut().push(::carve_state::Op::Delete { path });
                    }
                }
            }
        }
    };

    Ok(methods)
}

/// Generate AccessorOps trait implementation.
fn generate_accessor_ops_impl(accessor_name: &syn::Ident) -> syn::Result<TokenStream> {
    Ok(quote! {
        impl<'a> ::carve_state::AccessorOps for #accessor_name<'a> {
            fn has_changes(&self) -> bool {
                !self.ops.borrow().is_empty()
            }

            fn len(&self) -> usize {
                self.ops.borrow().len()
            }

            fn build(self) -> ::carve_state::Patch {
                ::carve_state::Patch::with_ops(self.ops.into_inner())
            }
        }
    })
}

/// Generate guard structs for nested accessors.
fn generate_nested_accessor_guards(
    _struct_name: &syn::Ident,
    fields: &[&FieldInput],
) -> syn::Result<TokenStream> {
    let mut guards = TokenStream::new();

    for field in fields {
        let kind = FieldKind::from_type(&field.ty, field.nested);
        if kind.is_nested() {
            // P0-3: Include field name in guard to avoid conflicts
            let field_name = field.ident();
            let pascal = to_pascal_case(&field_name.to_string());
            let guard_name = format_ident!("{}AccessorGuard", pascal);
            let field_ty = &field.ty;

            guards.extend(quote! {
                /// Guard for nested accessor that auto-merges on drop.
                pub struct #guard_name<'a> {
                    parent_ops: &'a ::std::cell::RefCell<Vec<::carve_state::Op>>,
                    // P0-2: Use trait associated type
                    accessor: <#field_ty as ::carve_state::CarveViewModel>::Accessor<'a>,
                }

                impl<'a> ::std::ops::Deref for #guard_name<'a> {
                    type Target = <#field_ty as ::carve_state::CarveViewModel>::Accessor<'a>;

                    fn deref(&self) -> &Self::Target {
                        &self.accessor
                    }
                }

                impl<'a> ::std::ops::DerefMut for #guard_name<'a> {
                    fn deref_mut(&mut self) -> &mut Self::Target {
                        &mut self.accessor
                    }
                }

                impl<'a> Drop for #guard_name<'a> {
                    fn drop(&mut self) {
                        use ::carve_state::AccessorOps;
                        // P1-6: Use drain instead of clone
                        // Access ops through the internal accessor method
                        let mut ops = self.accessor.ops_cell().borrow_mut();
                        if !ops.is_empty() {
                            self.parent_ops.borrow_mut().extend(ops.drain(..));
                        }
                    }
                }
            });
        }
    }

    Ok(guards)
}

/// Convert snake_case or camelCase to PascalCase.
fn to_pascal_case(s: &str) -> String {
    s.split('_')
        .filter(|s| !s.is_empty())
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                None => String::new(),
                Some(first) => {
                    first.to_uppercase().collect::<String>() + &chars.as_str().to_lowercase()
                }
            }
        })
        .collect()
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

/// Extract key and value types from Map<K, V>.
fn extract_map_types(ty: &syn::Type) -> (syn::Type, syn::Type) {
    if let syn::Type::Path(type_path) = ty {
        if let Some(segment) = type_path.path.segments.last() {
            if let syn::PathArguments::AngleBracketed(ab) = &segment.arguments {
                let mut iter = ab.args.iter();
                if let (
                    Some(syn::GenericArgument::Type(key)),
                    Some(syn::GenericArgument::Type(value)),
                ) = (iter.next(), iter.next())
                {
                    return (key.clone(), value.clone());
                }
            }
        }
    }
    // Fallback to String, Value
    (
        syn::parse_quote!(String),
        syn::parse_quote!(::serde_json::Value),
    )
}
