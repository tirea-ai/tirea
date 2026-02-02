//! Shared utility functions for code generation.

/// Extract the type name from a Type.
pub fn get_type_name(ty: &syn::Type) -> String {
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
pub fn extract_inner_type(ty: &syn::Type) -> syn::Type {
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
pub fn extract_map_types(ty: &syn::Type) -> (syn::Type, syn::Type) {
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

