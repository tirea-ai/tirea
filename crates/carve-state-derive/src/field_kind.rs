//! Field type analysis for code generation.

use syn::{GenericArgument, PathArguments, Type, TypePath};

/// The kind of a field, determining how to generate reader/writer methods.
#[derive(Debug, Clone, PartialEq)]
pub enum FieldKind {
    /// A primitive type (String, i32, bool, etc.)
    Primitive,

    /// An Option<T> type
    Option(Box<FieldKind>),

    /// A Vec<T> type
    Vec(Box<FieldKind>),

    /// A BTreeMap<K, V> or HashMap<K, V> type
    Map {
        key: Box<FieldKind>,
        value: Box<FieldKind>,
    },

    /// A nested CarveViewModel type
    Nested,
}

impl FieldKind {
    /// Analyze a type and determine its kind.
    pub fn from_type(ty: &Type, is_nested_attr: bool) -> Self {
        // If explicitly marked as nested, use Nested
        if is_nested_attr {
            return FieldKind::Nested;
        }

        match ty {
            Type::Path(type_path) => Self::from_type_path(type_path),
            _ => FieldKind::Primitive,
        }
    }

    fn from_type_path(type_path: &TypePath) -> Self {
        let path = &type_path.path;

        // Get the last segment (the actual type name)
        if let Some(segment) = path.segments.last() {
            let type_name = segment.ident.to_string();

            match type_name.as_str() {
                "Option" => {
                    if let Some(inner) = extract_single_generic_arg(&segment.arguments) {
                        FieldKind::Option(Box::new(Self::from_type(inner, false)))
                    } else {
                        FieldKind::Primitive
                    }
                }
                "Vec" => {
                    if let Some(inner) = extract_single_generic_arg(&segment.arguments) {
                        FieldKind::Vec(Box::new(Self::from_type(inner, false)))
                    } else {
                        FieldKind::Primitive
                    }
                }
                "BTreeMap" | "HashMap" => {
                    if let Some((key, value)) = extract_two_generic_args(&segment.arguments) {
                        FieldKind::Map {
                            key: Box::new(Self::from_type(key, false)),
                            value: Box::new(Self::from_type(value, false)),
                        }
                    } else {
                        FieldKind::Primitive
                    }
                }
                // Common primitive types
                "String" | "str" | "bool" | "char" | "i8" | "i16" | "i32" | "i64" | "i128"
                | "isize" | "u8" | "u16" | "u32" | "u64" | "u128" | "usize" | "f32" | "f64" => {
                    FieldKind::Primitive
                }
                // Unknown types - assume nested if it starts with uppercase (struct convention)
                _ => {
                    if segment.ident.to_string().chars().next().unwrap_or('a').is_uppercase() {
                        // Could be a nested struct - but we require explicit #[carve(nested)]
                        // for safety. Without it, treat as primitive (will use serde).
                        FieldKind::Primitive
                    } else {
                        FieldKind::Primitive
                    }
                }
            }
        } else {
            FieldKind::Primitive
        }
    }

    /// Check if this is a primitive type.
    #[allow(dead_code)]
    pub fn is_primitive(&self) -> bool {
        matches!(self, FieldKind::Primitive)
    }

    /// Check if this is an Option type.
    #[allow(dead_code)]
    pub fn is_option(&self) -> bool {
        matches!(self, FieldKind::Option(_))
    }

    /// Check if this is a Vec type.
    #[allow(dead_code)]
    pub fn is_vec(&self) -> bool {
        matches!(self, FieldKind::Vec(_))
    }

    /// Check if this is a Map type.
    #[allow(dead_code)]
    pub fn is_map(&self) -> bool {
        matches!(self, FieldKind::Map { .. })
    }

    /// Check if this is a nested type.
    pub fn is_nested(&self) -> bool {
        matches!(self, FieldKind::Nested)
    }
}

/// Extract a single generic type argument from path arguments.
fn extract_single_generic_arg(args: &PathArguments) -> Option<&Type> {
    match args {
        PathArguments::AngleBracketed(ab) => {
            if ab.args.len() == 1 {
                if let GenericArgument::Type(ty) = ab.args.first()? {
                    return Some(ty);
                }
            }
            None
        }
        _ => None,
    }
}

/// Extract two generic type arguments from path arguments (for Map types).
fn extract_two_generic_args(args: &PathArguments) -> Option<(&Type, &Type)> {
    match args {
        PathArguments::AngleBracketed(ab) => {
            if ab.args.len() == 2 {
                let mut iter = ab.args.iter();
                if let (
                    Some(GenericArgument::Type(key)),
                    Some(GenericArgument::Type(value)),
                ) = (iter.next(), iter.next())
                {
                    return Some((key, value));
                }
            }
            None
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use syn::parse_quote;

    #[test]
    fn test_primitive_types() {
        let ty: Type = parse_quote!(String);
        assert!(FieldKind::from_type(&ty, false).is_primitive());

        let ty: Type = parse_quote!(i32);
        assert!(FieldKind::from_type(&ty, false).is_primitive());

        let ty: Type = parse_quote!(bool);
        assert!(FieldKind::from_type(&ty, false).is_primitive());
    }

    #[test]
    fn test_option_type() {
        let ty: Type = parse_quote!(Option<String>);
        let kind = FieldKind::from_type(&ty, false);
        assert!(kind.is_option());

        if let FieldKind::Option(inner) = kind {
            assert!(inner.is_primitive());
        }
    }

    #[test]
    fn test_vec_type() {
        let ty: Type = parse_quote!(Vec<i32>);
        let kind = FieldKind::from_type(&ty, false);
        assert!(kind.is_vec());

        if let FieldKind::Vec(inner) = kind {
            assert!(inner.is_primitive());
        }
    }

    #[test]
    fn test_map_type() {
        let ty: Type = parse_quote!(BTreeMap<String, i32>);
        let kind = FieldKind::from_type(&ty, false);
        assert!(kind.is_map());
    }

    #[test]
    fn test_nested_attr() {
        let ty: Type = parse_quote!(Profile);
        let kind = FieldKind::from_type(&ty, true);
        assert!(kind.is_nested());
    }
}
