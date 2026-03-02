//! StateRef code generation.
//!
//! Generates typed state reference structs that use PatchSink for automatic
//! operation collection.

use super::utils::{extract_inner_type, extract_map_types, get_type_name};
use crate::field_kind::FieldKind;
use crate::parse::{FieldInput, ViewModelInput};
use proc_macro2::TokenStream;
use quote::{format_ident, quote};

/// Generate the StateRef struct and State trait implementation.
pub fn generate(input: &ViewModelInput) -> syn::Result<TokenStream> {
    let struct_name = &input.ident;
    let ref_name = format_ident!("{}Ref", struct_name);
    let vis = &input.vis;

    let fields: Vec<_> = input
        .fields()
        .into_iter()
        .filter(|f| f.is_included())
        .collect();

    let path_value = input.path.as_deref().unwrap_or("");

    let read_methods = generate_read_methods(&fields)?;
    let write_methods = generate_write_methods(&fields)?;
    let delete_methods = generate_delete_methods(&fields)?;
    let increment_methods = generate_increment_methods(&fields)?;
    let register_lattice_body = generate_register_lattice(path_value, &fields);
    let lattice_keys_body = generate_lattice_keys(&fields);
    let diff_ops_body = generate_diff_ops(&fields);
    let state_spec_impl = generate_state_spec(input)?;

    Ok(quote! {
        /// Typed state reference for reading and writing state.
        ///
        /// All modifications are automatically collected by the associated `PatchSink`.
        #vis struct #ref_name<'a> {
            doc: &'a ::tirea_state::DocCell,
            base: ::tirea_state::Path,
            sink: ::tirea_state::PatchSink<'a>,
        }

        impl<'a> #ref_name<'a> {
            /// Create a new state reference.
            #[doc(hidden)]
            pub fn new(
                doc: &'a ::tirea_state::DocCell,
                base: ::tirea_state::Path,
                sink: ::tirea_state::PatchSink<'a>,
            ) -> Self {
                Self { doc, base, sink }
            }

            #read_methods

            #write_methods

            #delete_methods

            #increment_methods
        }

        impl ::tirea_state::State for #struct_name {
            type Ref<'a> = #ref_name<'a>;

            const PATH: &'static str = #path_value;

            fn state_ref<'a>(
                doc: &'a ::tirea_state::DocCell,
                base: ::tirea_state::Path,
                sink: ::tirea_state::PatchSink<'a>,
            ) -> Self::Ref<'a> {
                #ref_name::new(doc, base, sink)
            }

            fn from_value(value: &::serde_json::Value) -> ::tirea_state::TireaResult<Self> {
                ::serde_json::from_value(value.clone())
                    .map_err(::tirea_state::TireaError::from)
            }

            fn to_value(&self) -> ::tirea_state::TireaResult<::serde_json::Value> {
                ::serde_json::to_value(self)
                    .map_err(::tirea_state::TireaError::from)
            }

            #register_lattice_body

            #lattice_keys_body

            #diff_ops_body
        }

        #state_spec_impl
    })
}

/// Generate read methods for each field.
fn generate_read_methods(fields: &[&FieldInput]) -> syn::Result<TokenStream> {
    let mut methods = TokenStream::new();

    for field in fields {
        let method = generate_read_method(field)?;
        methods.extend(method);
    }

    Ok(methods)
}

/// Generate a read method for a single field.
fn generate_read_method(field: &FieldInput) -> syn::Result<TokenStream> {
    let field_name = field.ident();
    let field_ty = &field.ty;
    let json_key = field.json_key();
    let kind = FieldKind::from_type(field_ty, field.flatten || field.nested, field.lattice);

    let method = match &kind {
        FieldKind::Nested => {
            if field.flatten {
                quote! {
                    /// Get a state reference for the flattened nested field.
                    pub fn #field_name(&self) -> <#field_ty as ::tirea_state::State>::Ref<'a> {
                        <#field_ty as ::tirea_state::State>::state_ref(
                            self.doc,
                            self.base.clone(),
                            self.sink.child(),
                        )
                    }
                }
            } else {
                quote! {
                    /// Get a state reference for the nested field.
                    pub fn #field_name(&self) -> <#field_ty as ::tirea_state::State>::Ref<'a> {
                        let path = self.base.clone().key(#json_key);
                        <#field_ty as ::tirea_state::State>::state_ref(
                            self.doc,
                            path,
                            self.sink.child(),
                        )
                    }
                }
            }
        }
        FieldKind::Option(inner) if inner.is_nested() => {
            let inner_ty = extract_inner_type(field_ty);
            let get_name = format_ident!("get_{}", field_name);
            quote! {
                /// Get a state reference for the optional nested field.
                pub fn #field_name(&self) -> <#inner_ty as ::tirea_state::State>::Ref<'a> {
                    let path = self.base.clone().key(#json_key);
                    <#inner_ty as ::tirea_state::State>::state_ref(
                        self.doc,
                        path,
                        self.sink.child(),
                    )
                }

                /// Read the optional nested field value.
                pub fn #get_name(&self) -> ::tirea_state::TireaResult<Option<#inner_ty>> {
                    let path = self.base.clone().key(#json_key);
                    match { let __guard = self.doc.get(); ::tirea_state::get_at_path(&__guard, &path).cloned() } {
                        None => Ok(None),
                        Some(value) if value.is_null() => Ok(None),
                        Some(value) => {
                            let v: #inner_ty = ::serde_json::from_value(value)
                                .map_err(::tirea_state::TireaError::from)?;
                            Ok(Some(v))
                        }
                    }
                }
            }
        }
        FieldKind::Option(_) => {
            quote! {
                /// Read the optional field value.
                pub fn #field_name(&self) -> ::tirea_state::TireaResult<#field_ty> {
                    let path = self.base.clone().key(#json_key);
                    match { let __guard = self.doc.get(); ::tirea_state::get_at_path(&__guard, &path).cloned() } {
                        None => Ok(None),
                        Some(value) if value.is_null() => Ok(None),
                        Some(value) => {
                            ::serde_json::from_value(value)
                                .map_err(::tirea_state::TireaError::from)
                        }
                    }
                }
            }
        }
        // Primitive, Lattice, Vec, and Map all use serde-based read with optional default
        FieldKind::Vec(_) | FieldKind::Map { .. } | FieldKind::Lattice | FieldKind::Primitive => {
            if let Some(default) = &field.default {
                let expr: syn::Expr = syn::parse_str(default).map_err(|e| {
                    syn::Error::new_spanned(field_ty, format!("invalid default expression: {}", e))
                })?;
                quote! {
                    /// Read the field value.
                    pub fn #field_name(&self) -> ::tirea_state::TireaResult<#field_ty> {
                        let path = self.base.clone().key(#json_key);
                        match { let __guard = self.doc.get(); ::tirea_state::get_at_path(&__guard, &path).cloned() } {
                            None => Ok(#expr),
                            Some(value) => {
                                ::serde_json::from_value(value)
                                    .map_err(::tirea_state::TireaError::from)
                            }
                        }
                    }
                }
            } else {
                quote! {
                    /// Read the field value.
                    pub fn #field_name(&self) -> ::tirea_state::TireaResult<#field_ty> {
                        let path = self.base.clone().key(#json_key);
                        let value = { let __guard = self.doc.get(); ::tirea_state::get_at_path(&__guard, &path).cloned() }
                            .ok_or_else(|| ::tirea_state::TireaError::path_not_found(path.clone()))?;
                        ::serde_json::from_value(value)
                            .map_err(::tirea_state::TireaError::from)
                    }
                }
            }
        }
    };

    Ok(method)
}

/// Generate write methods for each field.
fn generate_write_methods(fields: &[&FieldInput]) -> syn::Result<TokenStream> {
    let mut methods = TokenStream::new();

    for field in fields {
        let method = generate_write_method(field)?;
        methods.extend(method);
    }

    Ok(methods)
}

/// Generate a write method for a single field.
fn generate_write_method(field: &FieldInput) -> syn::Result<TokenStream> {
    let field_name = field.ident();
    let field_ty = &field.ty;
    let json_key = field.json_key();
    let kind = FieldKind::from_type(field_ty, field.flatten || field.nested, field.lattice);

    let methods = match &kind {
        FieldKind::Nested => {
            // Nested fields get their state ref through the read method
            quote! {}
        }
        FieldKind::Option(inner) if inner.is_nested() => {
            let set_name = format_ident!("set_{}", field_name);
            let none_name = format_ident!("{}_none", field_name);
            quote! {
                /// Set the entire optional nested field value.
                pub fn #set_name(&self, value: #field_ty) -> ::tirea_state::TireaResult<()> {
                    let path = self.base.clone().key(#json_key);
                    let value = ::serde_json::to_value(&value)
                        .map_err(::tirea_state::TireaError::from)?;
                    self.sink.collect(::tirea_state::Op::Set {
                        path,
                        value,
                    })
                }

                /// Set the optional field to null (None).
                pub fn #none_name(&self) -> ::tirea_state::TireaResult<()> {
                    let path = self.base.clone().key(#json_key);
                    self.sink.collect(::tirea_state::Op::Set {
                        path,
                        value: ::serde_json::Value::Null,
                    })
                }
            }
        }
        FieldKind::Option(_) => {
            let set_name = format_ident!("set_{}", field_name);
            let none_name = format_ident!("{}_none", field_name);
            quote! {
                /// Set the optional field value.
                pub fn #set_name(&self, value: #field_ty) -> ::tirea_state::TireaResult<()> {
                    let path = self.base.clone().key(#json_key);
                    let value = ::serde_json::to_value(&value)
                        .map_err(::tirea_state::TireaError::from)?;
                    self.sink.collect(::tirea_state::Op::Set {
                        path,
                        value,
                    })
                }

                /// Set the optional field to null (None).
                pub fn #none_name(&self) -> ::tirea_state::TireaResult<()> {
                    let path = self.base.clone().key(#json_key);
                    self.sink.collect(::tirea_state::Op::Set {
                        path,
                        value: ::serde_json::Value::Null,
                    })
                }
            }
        }
        FieldKind::Vec(inner) => {
            let inner_ty = extract_inner_type(field_ty);
            let set_name = format_ident!("set_{}", field_name);
            let push_name = format_ident!("{}_push", field_name);

            if inner.is_nested() {
                quote! {
                    /// Set the entire array.
                    pub fn #set_name(&self, value: #field_ty) -> ::tirea_state::TireaResult<()> {
                        let path = self.base.clone().key(#json_key);
                        let value = ::serde_json::to_value(&value)
                            .map_err(::tirea_state::TireaError::from)?;
                        self.sink.collect(::tirea_state::Op::Set {
                            path,
                            value,
                        })
                    }

                    /// Push an item to the array.
                    pub fn #push_name(&self, value: #inner_ty) -> ::tirea_state::TireaResult<()> {
                        let path = self.base.clone().key(#json_key);
                        let value = ::serde_json::to_value(&value)
                            .map_err(::tirea_state::TireaError::from)?;
                        self.sink.collect(::tirea_state::Op::Append {
                            path,
                            value,
                        })
                    }
                }
            } else {
                quote! {
                    /// Set the entire array.
                    pub fn #set_name(&self, value: #field_ty) -> ::tirea_state::TireaResult<()> {
                        let path = self.base.clone().key(#json_key);
                        let value = ::serde_json::to_value(&value)
                            .map_err(::tirea_state::TireaError::from)?;
                        self.sink.collect(::tirea_state::Op::Set {
                            path,
                            value,
                        })
                    }

                    /// Push an item to the array.
                    pub fn #push_name(&self, value: impl Into<#inner_ty>) -> ::tirea_state::TireaResult<()> {
                        let path = self.base.clone().key(#json_key);
                        let v: #inner_ty = value.into();
                        let value = ::serde_json::to_value(&v)
                            .map_err(::tirea_state::TireaError::from)?;
                        self.sink.collect(::tirea_state::Op::Append {
                            path,
                            value,
                        })
                    }
                }
            }
        }
        FieldKind::Map { .. } => {
            let (key_ty, value_ty) = extract_map_types(field_ty);
            let key_type_name = get_type_name(&key_ty);
            let set_name = format_ident!("set_{}", field_name);

            let insert_method = if key_type_name == "String" {
                let insert_name = format_ident!("{}_insert", field_name);
                quote! {
                    /// Insert a key-value pair into the map.
                    pub fn #insert_name(&self, key: impl Into<String>, value: impl Into<#value_ty>) -> ::tirea_state::TireaResult<()> {
                        let path = self.base.clone().key(#json_key);
                        let k: String = key.into();
                        let path = path.key(k);
                        let v: #value_ty = value.into();
                        let value = ::serde_json::to_value(&v)
                            .map_err(::tirea_state::TireaError::from)?;
                        self.sink.collect(::tirea_state::Op::Set {
                            path,
                            value,
                        })
                    }
                }
            } else {
                quote! {}
            };

            quote! {
                /// Set the entire map.
                pub fn #set_name(&self, value: #field_ty) -> ::tirea_state::TireaResult<()> {
                    let path = self.base.clone().key(#json_key);
                    let value = ::serde_json::to_value(&value)
                        .map_err(::tirea_state::TireaError::from)?;
                    self.sink.collect(::tirea_state::Op::Set {
                        path,
                        value,
                    })
                }

                #insert_method
            }
        }
        FieldKind::Lattice => {
            let set_name = format_ident!("set_{}", field_name);
            let merge_name = format_ident!("merge_{}", field_name);
            quote! {
                /// Set the lattice field value.
                pub fn #set_name(&self, value: #field_ty) -> ::tirea_state::TireaResult<()> {
                    let path = self.base.clone().key(#json_key);
                    let value = ::serde_json::to_value(&value)
                        .map_err(::tirea_state::TireaError::from)?;
                    self.sink.collect(::tirea_state::Op::Set { path, value })
                }

                /// Merge a lattice delta into this field.
                ///
                /// Emits an `Op::LatticeMerge` with the delta. The actual merge is
                /// deferred to apply time via `LatticeRegistry`.
                pub fn #merge_name(&self, other: &#field_ty) -> ::tirea_state::TireaResult<()> {
                    let path = self.base.clone().key(#json_key);
                    let value = ::serde_json::to_value(other)
                        .map_err(::tirea_state::TireaError::from)?;
                    self.sink.collect(::tirea_state::Op::LatticeMerge { path, value })
                }
            }
        }
        FieldKind::Primitive => {
            let set_name = format_ident!("set_{}", field_name);
            let type_name = get_type_name(field_ty);

            if type_name == "String" {
                quote! {
                    /// Set the field value.
                    pub fn #set_name(&self, value: impl Into<String>) -> ::tirea_state::TireaResult<()> {
                        let path = self.base.clone().key(#json_key);
                        let v: String = value.into();
                        self.sink.collect(::tirea_state::Op::Set {
                            path,
                            value: ::serde_json::Value::String(v),
                        })
                    }
                }
            } else {
                quote! {
                    /// Set the field value.
                    pub fn #set_name(&self, value: #field_ty) -> ::tirea_state::TireaResult<()> {
                        let path = self.base.clone().key(#json_key);
                        let value = ::serde_json::to_value(&value)
                            .map_err(::tirea_state::TireaError::from)?;
                        self.sink.collect(::tirea_state::Op::Set {
                            path,
                            value,
                        })
                    }
                }
            }
        }
    };

    Ok(methods)
}

/// Generate delete methods for each field.
fn generate_delete_methods(fields: &[&FieldInput]) -> syn::Result<TokenStream> {
    let mut methods = TokenStream::new();

    for field in fields {
        let field_name = field.ident();
        let json_key = field.json_key();
        let delete_name = format_ident!("delete_{}", field_name);

        // Skip delete for flatten fields
        if field.flatten {
            continue;
        }

        methods.extend(quote! {
            /// Delete this field entirely from the object.
            pub fn #delete_name(&self) -> ::tirea_state::TireaResult<()> {
                let path = self.base.clone().key(#json_key);
                self.sink.collect(::tirea_state::Op::Delete { path })
            }
        });
    }

    Ok(methods)
}

/// Generate increment methods for numeric fields.
fn generate_increment_methods(fields: &[&FieldInput]) -> syn::Result<TokenStream> {
    let mut methods = TokenStream::new();

    for field in fields {
        let field_ty = &field.ty;
        let kind = FieldKind::from_type(field_ty, field.flatten || field.nested, field.lattice);

        if !matches!(kind, FieldKind::Primitive) {
            continue;
        }

        let type_name = get_type_name(field_ty);

        let (is_int, is_float) = match type_name.as_str() {
            "i32" | "i64" => (true, false),
            "f64" => (false, true),
            _ => continue,
        };

        let field_name = field.ident();
        let json_key = field.json_key();
        let increment_name = format_ident!("increment_{}", field_name);
        let decrement_name = format_ident!("decrement_{}", field_name);

        if is_int {
            methods.extend(quote! {
                /// Increment the numeric field by the given amount.
                pub fn #increment_name(&self, amount: i64) -> ::tirea_state::TireaResult<()> {
                    let path = self.base.clone().key(#json_key);
                    self.sink.collect(::tirea_state::Op::Increment {
                        path,
                        amount: ::tirea_state::Number::Int(amount),
                    })
                }

                /// Decrement the numeric field by the given amount.
                pub fn #decrement_name(&self, amount: i64) -> ::tirea_state::TireaResult<()> {
                    let path = self.base.clone().key(#json_key);
                    self.sink.collect(::tirea_state::Op::Decrement {
                        path,
                        amount: ::tirea_state::Number::Int(amount),
                    })
                }
            });
        } else if is_float {
            methods.extend(quote! {
                /// Increment the numeric field by the given amount.
                pub fn #increment_name(&self, amount: f64) -> ::tirea_state::TireaResult<()> {
                    let path = self.base.clone().key(#json_key);
                    self.sink.collect(::tirea_state::Op::Increment {
                        path,
                        amount: ::tirea_state::Number::Float(amount),
                    })
                }

                /// Decrement the numeric field by the given amount.
                pub fn #decrement_name(&self, amount: f64) -> ::tirea_state::TireaResult<()> {
                    let path = self.base.clone().key(#json_key);
                    self.sink.collect(::tirea_state::Op::Decrement {
                        path,
                        amount: ::tirea_state::Number::Float(amount),
                    })
                }
            });
        }
    }

    Ok(methods)
}

/// Generate `register_lattice` override for lattice fields.
///
/// Returns empty tokens if there are no lattice fields (inherits the default no-op).
fn generate_register_lattice(base_path: &str, fields: &[&FieldInput]) -> TokenStream {
    let lattice_fields: Vec<_> = fields.iter().filter(|f| f.lattice).collect();

    if lattice_fields.is_empty() || base_path.is_empty() {
        return TokenStream::new();
    }

    let registrations: Vec<TokenStream> = lattice_fields
        .iter()
        .map(|field| {
            let field_ty = &field.ty;
            let json_key = field.json_key();
            let full_path = format!("{}.{}", base_path, json_key);
            quote! {
                registry.register::<#field_ty>(::tirea_state::parse_path(#full_path));
            }
        })
        .collect();

    quote! {
        fn register_lattice(registry: &mut ::tirea_state::LatticeRegistry) {
            #(#registrations)*
        }
    }
}

/// Generate `lattice_keys` override for lattice fields.
///
/// Returns empty tokens if there are no lattice fields (inherits the default `&[]`).
fn generate_lattice_keys(fields: &[&FieldInput]) -> TokenStream {
    let lattice_fields: Vec<_> = fields.iter().filter(|f| f.lattice).collect();

    if lattice_fields.is_empty() {
        return TokenStream::new();
    }

    let keys: Vec<String> = lattice_fields.iter().map(|f| f.json_key()).collect();

    quote! {
        fn lattice_keys() -> &'static [&'static str] {
            &[#(#keys),*]
        }
    }
}

/// Generate `diff_ops` override that does per-field comparison.
///
/// For each included field:
/// - **Lattice** fields: `PartialEq` comparison, emit `Op::LatticeMerge` when changed
/// - **Nested** (non-flatten) fields: recursively call `T::diff_ops`
/// - **Flatten** fields: recursively call `T::diff_ops` at the same base path
/// - **All others** (Primitive, Vec, Map, Option): serialize both values and compare
///   at the `serde_json::Value` level, emit `Op::Set` when changed
///
/// Lattice types all derive `PartialEq`, so they can use direct comparison to
/// avoid serialization when unchanged. Non-lattice fields may not implement
/// `PartialEq` (e.g., containing `Thread`), so we compare serialized values.
fn generate_diff_ops(fields: &[&FieldInput]) -> TokenStream {
    let mut field_checks = Vec::new();

    for field in fields {
        let field_name = field.ident();
        let field_ty = &field.ty;
        let json_key = field.json_key();
        let kind = FieldKind::from_type(field_ty, field.flatten || field.nested, field.lattice);

        // #[serde(flatten)] fields have their keys merged into the parent
        // object, so we diff each merged key at the base_path level.
        if field.has_serde_flatten() {
            field_checks.push(quote! {
                {
                    let old_flat = ::serde_json::to_value(&old.#field_name)
                        .map_err(::tirea_state::TireaError::from)?;
                    let new_flat = ::serde_json::to_value(&new.#field_name)
                        .map_err(::tirea_state::TireaError::from)?;
                    if old_flat != new_flat {
                        let empty = ::serde_json::Map::new();
                        let old_obj = old_flat.as_object().unwrap_or(&empty);
                        let new_obj = new_flat.as_object().unwrap_or(&empty);
                        for (key, new_val) in new_obj {
                            if old_obj.get(key) != Some(new_val) {
                                ops.push(::tirea_state::Op::set(
                                    base_path.clone().key(key),
                                    new_val.clone(),
                                ));
                            }
                        }
                        for key in old_obj.keys() {
                            if !new_obj.contains_key(key) {
                                ops.push(::tirea_state::Op::delete(base_path.clone().key(key)));
                            }
                        }
                    }
                }
            });
            continue;
        }

        let check = match &kind {
            FieldKind::Nested if field.flatten => {
                // #[tirea(flatten)]: delegate to inner type's diff_ops at the same base path
                quote! {
                    ops.extend(
                        <#field_ty as ::tirea_state::State>::diff_ops(
                            &old.#field_name, &new.#field_name, base_path,
                        )?
                    );
                }
            }
            FieldKind::Nested => {
                // Nested: delegate to inner type's diff_ops at sub-path
                quote! {
                    {
                        let sub_path = base_path.clone().key(#json_key);
                        ops.extend(
                            <#field_ty as ::tirea_state::State>::diff_ops(
                                &old.#field_name, &new.#field_name, &sub_path,
                            )?
                        );
                    }
                }
            }
            FieldKind::Lattice => {
                // Lattice CRDT field: PartialEq + LatticeMerge
                // All lattice types (GCounter, GSet, ORSet, etc.) derive PartialEq.
                quote! {
                    if old.#field_name != new.#field_name {
                        let field_path = base_path.clone().key(#json_key);
                        let value = ::serde_json::to_value(&new.#field_name)
                            .map_err(::tirea_state::TireaError::from)?;
                        ops.push(::tirea_state::Op::lattice_merge(field_path, value));
                    }
                }
            }
            _ => {
                // Primitive, Option, Vec, Map: serialize and compare Values.
                // We serialize both sides because field types may not impl PartialEq.
                quote! {
                    {
                        let old_val = ::serde_json::to_value(&old.#field_name)
                            .map_err(::tirea_state::TireaError::from)?;
                        let new_val = ::serde_json::to_value(&new.#field_name)
                            .map_err(::tirea_state::TireaError::from)?;
                        if old_val != new_val {
                            let field_path = base_path.clone().key(#json_key);
                            ops.push(::tirea_state::Op::set(field_path, new_val));
                        }
                    }
                }
            }
        };

        field_checks.push(check);
    }

    quote! {
        fn diff_ops(
            old: &Self,
            new: &Self,
            base_path: &::tirea_state::Path,
        ) -> ::tirea_state::TireaResult<Vec<::tirea_state::Op>> {
            let mut ops = Vec::new();
            #(#field_checks)*
            Ok(ops)
        }
    }
}

/// Generate `impl StateSpec` when `#[tirea(action = "...")]` is present.
///
/// The generated impl delegates `reduce` to an inherent method on the struct,
/// so the user writes business logic as `impl MyState { fn reduce(...) { ... } }`.
fn generate_state_spec(input: &ViewModelInput) -> syn::Result<TokenStream> {
    let action_str = match &input.action {
        Some(a) => a,
        None => return Ok(TokenStream::new()),
    };

    let struct_name = &input.ident;
    let action_ty: syn::Type = syn::parse_str(action_str).map_err(|e| {
        syn::Error::new_spanned(
            &input.ident,
            format!(
                "invalid action type in #[tirea(action = \"{}\")]: {}",
                action_str, e
            ),
        )
    })?;

    Ok(quote! {
        impl ::tirea_state::StateSpec for #struct_name {
            type Action = #action_ty;

            fn reduce(&mut self, action: Self::Action) {
                #struct_name::reduce(self, action)
            }
        }
    })
}
