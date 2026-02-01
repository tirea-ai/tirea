//! Writer code generation.

use crate::field_kind::FieldKind;
use crate::parse::{FieldInput, ViewModelInput};
use proc_macro2::TokenStream;
use quote::{format_ident, quote};

/// Generate the writer struct and impl.
pub fn generate(input: &ViewModelInput) -> syn::Result<TokenStream> {
    let struct_name = &input.ident;
    let writer_name = format_ident!("{}Writer", struct_name);
    let vis = &input.vis;

    let fields: Vec<_> = input.fields().into_iter().filter(|f| f.is_included()).collect();

    let field_setters = generate_field_setters(&fields)?;
    let delete_methods = generate_delete_methods(&fields)?;
    let nested_guards = generate_nested_guards(struct_name, &fields)?;

    Ok(quote! {
        /// Strongly-typed writer for modifying fields.
        #vis struct #writer_name {
            base: ::carve_state::Path,
            ops: Vec<::carve_state::Op>,
        }

        impl #writer_name {
            /// Create a new writer at the specified base path.
            #[doc(hidden)]
            pub fn new(base: ::carve_state::Path) -> Self {
                Self {
                    base,
                    ops: Vec::new(),
                }
            }

            #field_setters

            #delete_methods

            /// Consume and build a patch.
            pub fn build(self) -> ::carve_state::Patch {
                ::carve_state::Patch::with_ops(self.ops)
            }

            /// Check if this writer has any operations.
            pub fn is_empty(&self) -> bool {
                self.ops.is_empty()
            }

            /// Get the number of operations.
            pub fn len(&self) -> usize {
                self.ops.len()
            }
        }

        impl ::carve_state::WriterOps for #writer_name {
            fn ops(&self) -> &[::carve_state::Op] {
                &self.ops
            }

            fn ops_mut(&mut self) -> &mut Vec<::carve_state::Op> {
                &mut self.ops
            }

            fn take_ops(&mut self) -> Vec<::carve_state::Op> {
                std::mem::take(&mut self.ops)
            }

            fn into_patch(self) -> ::carve_state::Patch {
                self.build()
            }
        }

        #nested_guards
    })
}

/// Generate setter methods for each field.
fn generate_field_setters(fields: &[&FieldInput]) -> syn::Result<TokenStream> {
    let mut methods = TokenStream::new();

    for field in fields {
        let setter = generate_field_setter(field)?;
        methods.extend(setter);
    }

    Ok(methods)
}

/// Generate a setter method for a single field.
fn generate_field_setter(field: &FieldInput) -> syn::Result<TokenStream> {
    let field_name = field.ident();
    let field_ty = &field.ty;
    let json_key = field.json_key();
    let kind = FieldKind::from_type(field_ty, field.nested);

    let methods = match &kind {
        FieldKind::Nested => {
            // P0-3: Guard name includes field name to avoid conflicts
            // when same nested type appears multiple times
            // Convert field_name to UpperCamelCase for guard name
            let guard_name = {
                let field_str = field_name.to_string();
                let pascal = to_pascal_case(&field_str);
                format_ident!("{}WriterGuard", pascal)
            };
            quote! {
                /// Get a writer for the nested field.
                ///
                /// The nested writer's operations are automatically merged
                /// into the parent when the guard is dropped.
                pub fn #field_name(&mut self) -> #guard_name<'_> {
                    let mut path = self.base.clone();
                    path.push_key(#json_key);
                    #guard_name {
                        parent_ops: &mut self.ops,
                        // P0-2: Use trait associated type instead of string-based type name
                        writer: <#field_ty as ::carve_state::CarveViewModel>::writer(path),
                    }
                }
            }
        }
        FieldKind::Option(inner) if inner.is_nested() => {
            // Option<Nested>: provide guard for fine-grained patching + set/none methods
            let inner_ty = extract_inner_type(field_ty);
            let guard_name = {
                let field_str = field_name.to_string();
                let pascal = to_pascal_case(&field_str);
                format_ident!("{}WriterGuard", pascal)
            };
            let none_name = format_ident!("{}_none", field_name);
            let set_name = format_ident!("set_{}", field_name);

            quote! {
                /// Get a writer for the optional nested field.
                ///
                /// This allows fine-grained patching of the nested structure.
                /// The nested writer's operations are automatically merged
                /// into the parent when the guard is dropped.
                pub fn #field_name(&mut self) -> #guard_name<'_> {
                    let mut path = self.base.clone();
                    path.push_key(#json_key);
                    #guard_name {
                        parent_ops: &mut self.ops,
                        writer: <#inner_ty as ::carve_state::CarveViewModel>::writer(path),
                    }
                }

                /// Set the entire optional field value.
                pub fn #set_name(&mut self, value: #field_ty) -> &mut Self {
                    let mut path = self.base.clone();
                    path.push_key(#json_key);
                    self.ops.push(::carve_state::Op::Set {
                        path,
                        value: ::serde_json::to_value(&value).unwrap_or(::serde_json::Value::Null),
                    });
                    self
                }

                /// Set the optional field to null (None).
                pub fn #none_name(&mut self) -> &mut Self {
                    let mut path = self.base.clone();
                    path.push_key(#json_key);
                    self.ops.push(::carve_state::Op::Set {
                        path,
                        value: ::serde_json::Value::Null,
                    });
                    self
                }
            }
        }
        FieldKind::Option(_) => {
            let none_name = format_ident!("{}_none", field_name);
            quote! {
                /// Set the optional field value.
                pub fn #field_name(&mut self, value: #field_ty) -> &mut Self {
                    let mut path = self.base.clone();
                    path.push_key(#json_key);
                    self.ops.push(::carve_state::Op::Set {
                        path,
                        value: ::serde_json::to_value(&value).unwrap_or(::serde_json::Value::Null),
                    });
                    self
                }

                /// Set the optional field to null (None).
                ///
                /// This is different from `delete_*()`: this sets the field to JSON `null`,
                /// while `delete_*()` removes the field entirely from the object.
                pub fn #none_name(&mut self) -> &mut Self {
                    let mut path = self.base.clone();
                    path.push_key(#json_key);
                    self.ops.push(::carve_state::Op::Set {
                        path,
                        value: ::serde_json::Value::Null,
                    });
                    self
                }
            }
        }
        FieldKind::Vec(inner) => {
            let inner_ty = extract_inner_type(field_ty);
            let push_name = format_ident!("{}_push", field_name);

            // For Vec<Nested>, we need different handling
            if inner.is_nested() {
                quote! {
                    /// Set the entire array.
                    pub fn #field_name(&mut self, value: #field_ty) -> &mut Self {
                        let mut path = self.base.clone();
                        path.push_key(#json_key);
                        self.ops.push(::carve_state::Op::Set {
                            path,
                            value: ::serde_json::to_value(&value).unwrap_or(::serde_json::Value::Null),
                        });
                        self
                    }

                    /// Push an item to the array.
                    pub fn #push_name(&mut self, value: #inner_ty) -> &mut Self {
                        let mut path = self.base.clone();
                        path.push_key(#json_key);
                        self.ops.push(::carve_state::Op::Append {
                            path,
                            value: ::serde_json::to_value(&value).unwrap_or(::serde_json::Value::Null),
                        });
                        self
                    }
                }
            } else {
                quote! {
                    /// Set the entire array.
                    pub fn #field_name(&mut self, value: #field_ty) -> &mut Self {
                        let mut path = self.base.clone();
                        path.push_key(#json_key);
                        self.ops.push(::carve_state::Op::Set {
                            path,
                            value: ::serde_json::to_value(&value).unwrap_or(::serde_json::Value::Null),
                        });
                        self
                    }

                    /// Push an item to the array.
                    pub fn #push_name(&mut self, value: impl Into<#inner_ty>) -> &mut Self {
                        let mut path = self.base.clone();
                        path.push_key(#json_key);
                        let v: #inner_ty = value.into();
                        self.ops.push(::carve_state::Op::Append {
                            path,
                            value: ::serde_json::to_value(&v).unwrap_or(::serde_json::Value::Null),
                        });
                        self
                    }
                }
            }
        }
        FieldKind::Map { .. } => {
            let (key_ty, value_ty) = extract_map_types(field_ty);
            let key_type_name = get_type_name(&key_ty);

            // Only generate insert method for String keys (JSON object keys must be strings)
            let insert_method = if key_type_name == "String" {
                let insert_name = format_ident!("{}_insert", field_name);
                quote! {
                    /// Insert a key-value pair into the map.
                    pub fn #insert_name(&mut self, key: impl Into<String>, value: impl Into<#value_ty>) -> &mut Self {
                        let mut path = self.base.clone();
                        path.push_key(#json_key);
                        let k: String = key.into();
                        path.push_key(k);
                        let v: #value_ty = value.into();
                        self.ops.push(::carve_state::Op::Set {
                            path,
                            value: ::serde_json::to_value(&v).unwrap_or(::serde_json::Value::Null),
                        });
                        self
                    }
                }
            } else {
                // Non-String key maps: only support setting entire map
                quote! {}
            };

            quote! {
                /// Set the entire map.
                pub fn #field_name(&mut self, value: #field_ty) -> &mut Self {
                    let mut path = self.base.clone();
                    path.push_key(#json_key);
                    self.ops.push(::carve_state::Op::Set {
                        path,
                        value: ::serde_json::to_value(&value).unwrap_or(::serde_json::Value::Null),
                    });
                    self
                }

                #insert_method
            }
        }
        FieldKind::Primitive => {
            // Check if the type is String for impl Into support
            let type_name = get_type_name(field_ty);
            if type_name == "String" {
                quote! {
                    /// Set the field value.
                    pub fn #field_name(&mut self, value: impl Into<String>) -> &mut Self {
                        let mut path = self.base.clone();
                        path.push_key(#json_key);
                        let v: String = value.into();
                        self.ops.push(::carve_state::Op::Set {
                            path,
                            value: ::serde_json::Value::String(v),
                        });
                        self
                    }
                }
            } else {
                quote! {
                    /// Set the field value.
                    pub fn #field_name(&mut self, value: #field_ty) -> &mut Self {
                        let mut path = self.base.clone();
                        path.push_key(#json_key);
                        self.ops.push(::carve_state::Op::Set {
                            path,
                            value: ::serde_json::to_value(&value).unwrap_or(::serde_json::Value::Null),
                        });
                        self
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
        let delete_name = format_ident!("delete_{}", field_name);
        let json_key = field.json_key();

        methods.extend(quote! {
            /// Delete this field entirely from the object.
            ///
            /// This removes the field from the JSON object. For `Option` fields,
            /// use `*_none()` to set the value to `null` instead.
            pub fn #delete_name(&mut self) -> &mut Self {
                let mut path = self.base.clone();
                path.push_key(#json_key);
                self.ops.push(::carve_state::Op::Delete { path });
                self
            }
        });
    }

    Ok(methods)
}

/// Generate guard structs for nested writers.
fn generate_nested_guards(struct_name: &syn::Ident, fields: &[&FieldInput]) -> syn::Result<TokenStream> {
    let mut guards = TokenStream::new();

    for field in fields {
        let kind = FieldKind::from_type(&field.ty, field.nested);

        // Generate guard for both Nested and Option<Nested>
        let needs_guard = match &kind {
            FieldKind::Nested => true,
            FieldKind::Option(inner) if inner.is_nested() => true,
            _ => false,
        };

        if needs_guard {
            // P0-3: Include field name in guard to avoid conflicts
            // when same nested type appears multiple times
            let field_name = field.ident();
            let pascal = to_pascal_case(&field_name.to_string());
            let guard_name = format_ident!("{}WriterGuard", pascal);

            // For Option<Nested>, use the inner type
            let inner_ty = match &kind {
                FieldKind::Option(_) => extract_inner_type(&field.ty),
                _ => field.ty.clone(),
            };
            let field_ty = &inner_ty;

            guards.extend(quote! {
                /// Guard for nested writer that auto-merges on drop.
                pub struct #guard_name<'a> {
                    parent_ops: &'a mut Vec<::carve_state::Op>,
                    // P0-2: Use trait associated type
                    writer: <#field_ty as ::carve_state::CarveViewModel>::Writer,
                }

                impl<'a> std::ops::Deref for #guard_name<'a> {
                    type Target = <#field_ty as ::carve_state::CarveViewModel>::Writer;

                    fn deref(&self) -> &Self::Target {
                        &self.writer
                    }
                }

                impl<'a> std::ops::DerefMut for #guard_name<'a> {
                    fn deref_mut(&mut self) -> &mut Self::Target {
                        &mut self.writer
                    }
                }

                impl<'a> Drop for #guard_name<'a> {
                    fn drop(&mut self) {
                        use ::carve_state::WriterOps;
                        // P1-6: Use take_ops (drain) instead of cloning
                        self.parent_ops.extend(self.writer.take_ops());
                    }
                }
            });
        }
    }

    // Suppress unused warning if no nested fields
    let _ = struct_name;

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
