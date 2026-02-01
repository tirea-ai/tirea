//! Error types for carve-state operations.

use crate::Path;
use thiserror::Error;

/// Result type alias for carve-state operations.
pub type CarveResult<T> = Result<T, CarveError>;

/// Errors that can occur during carve-state operations.
#[derive(Debug, Error)]
pub enum CarveError {
    /// Path does not exist in the document.
    #[error("path not found: {path}")]
    PathNotFound {
        /// The path that was not found.
        path: Path,
    },

    /// Array index is out of bounds.
    #[error("index {index} out of bounds (len: {len}) at path {path}")]
    IndexOutOfBounds {
        /// The path to the array.
        path: Path,
        /// The index that was accessed.
        index: usize,
        /// The actual length of the array.
        len: usize,
    },

    /// Type mismatch when accessing a value.
    #[error("type mismatch at {path}: expected {expected}, found {found}")]
    TypeMismatch {
        /// The path where the mismatch occurred.
        path: Path,
        /// The expected type.
        expected: &'static str,
        /// The actual type found.
        found: &'static str,
    },

    /// Numeric operation on a non-numeric value.
    #[error("numeric operation requires number at {path}")]
    NumericOperationOnNonNumber {
        /// The path where the non-numeric value was found.
        path: Path,
    },

    /// Merge operation requires an object value.
    #[error("merge requires object value at {path}")]
    MergeRequiresObject {
        /// The path where a non-object was found.
        path: Path,
    },

    /// Append operation requires an array value.
    #[error("append requires array value at {path}")]
    AppendRequiresArray {
        /// The path where a non-array was found.
        path: Path,
    },

    /// Invalid operation error.
    #[error("invalid operation: {message}")]
    InvalidOperation {
        /// Description of what went wrong.
        message: String,
    },

    /// JSON serialization/deserialization error.
    #[error("serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
}

impl CarveError {
    /// Create a path not found error.
    #[inline]
    pub fn path_not_found(path: Path) -> Self {
        CarveError::PathNotFound { path }
    }

    /// Create an index out of bounds error.
    #[inline]
    pub fn index_out_of_bounds(path: Path, index: usize, len: usize) -> Self {
        CarveError::IndexOutOfBounds { path, index, len }
    }

    /// Create a type mismatch error.
    #[inline]
    pub fn type_mismatch(path: Path, expected: &'static str, found: &'static str) -> Self {
        CarveError::TypeMismatch {
            path,
            expected,
            found,
        }
    }

    /// Create a numeric operation on non-number error.
    #[inline]
    pub fn numeric_on_non_number(path: Path) -> Self {
        CarveError::NumericOperationOnNonNumber { path }
    }

    /// Create a merge requires object error.
    #[inline]
    pub fn merge_requires_object(path: Path) -> Self {
        CarveError::MergeRequiresObject { path }
    }

    /// Create an append requires array error.
    #[inline]
    pub fn append_requires_array(path: Path) -> Self {
        CarveError::AppendRequiresArray { path }
    }

    /// Create an invalid operation error.
    #[inline]
    pub fn invalid_operation(message: impl Into<String>) -> Self {
        CarveError::InvalidOperation {
            message: message.into(),
        }
    }
}

/// Get the type name of a JSON value.
#[inline]
pub fn value_type_name(v: &serde_json::Value) -> &'static str {
    match v {
        serde_json::Value::Null => "null",
        serde_json::Value::Bool(_) => "boolean",
        serde_json::Value::Number(_) => "number",
        serde_json::Value::String(_) => "string",
        serde_json::Value::Array(_) => "array",
        serde_json::Value::Object(_) => "object",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::path;

    #[test]
    fn test_error_display() {
        let err = CarveError::path_not_found(path!("users", 0, "name"));
        assert!(err.to_string().contains("path not found"));
    }

    #[test]
    fn test_value_type_name() {
        use serde_json::json;

        assert_eq!(value_type_name(&json!(null)), "null");
        assert_eq!(value_type_name(&json!(true)), "boolean");
        assert_eq!(value_type_name(&json!(42)), "number");
        assert_eq!(value_type_name(&json!("hello")), "string");
        assert_eq!(value_type_name(&json!([1, 2, 3])), "array");
        assert_eq!(value_type_name(&json!({"a": 1})), "object");
    }
}
