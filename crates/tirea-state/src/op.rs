//! Patch operations for modifying JSON documents.
//!
//! Each operation describes a single atomic change to apply to a document.

use crate::Path;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// A numeric value that can be used in increment/decrement operations.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Number {
    /// Integer value.
    Int(i64),
    /// Floating-point value.
    Float(f64),
}

impl Number {
    /// Create an integer number.
    #[inline]
    pub fn int(v: i64) -> Self {
        Number::Int(v)
    }

    /// Create a floating-point number.
    #[inline]
    pub fn float(v: f64) -> Self {
        Number::Float(v)
    }

    /// Convert to f64.
    #[inline]
    pub fn as_f64(&self) -> f64 {
        match self {
            Number::Int(i) => *i as f64,
            Number::Float(f) => *f,
        }
    }

    /// Convert to i64 (truncates floats).
    #[inline]
    pub fn as_i64(&self) -> i64 {
        match self {
            Number::Int(i) => *i,
            Number::Float(f) => *f as i64,
        }
    }

    /// Check if this is an integer.
    #[inline]
    pub fn is_int(&self) -> bool {
        matches!(self, Number::Int(_))
    }

    /// Check if this is a float.
    #[inline]
    pub fn is_float(&self) -> bool {
        matches!(self, Number::Float(_))
    }
}

impl From<i64> for Number {
    fn from(v: i64) -> Self {
        Number::Int(v)
    }
}

impl From<i32> for Number {
    fn from(v: i32) -> Self {
        Number::Int(v as i64)
    }
}

impl From<u32> for Number {
    fn from(v: u32) -> Self {
        Number::Int(v as i64)
    }
}

impl From<u64> for Number {
    fn from(v: u64) -> Self {
        Number::Int(v as i64)
    }
}

impl From<f64> for Number {
    fn from(v: f64) -> Self {
        Number::Float(v)
    }
}

impl From<f32> for Number {
    fn from(v: f32) -> Self {
        Number::Float(v as f64)
    }
}

/// A single patch operation.
///
/// Operations are the atomic units of change. Each operation targets a specific
/// path in the document and performs a specific mutation.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Op {
    /// Set a value at the path.
    ///
    /// Creates intermediate objects if they don't exist.
    /// Returns error if array index is out of bounds.
    Set {
        /// Target path.
        path: Path,
        /// Value to set.
        value: Value,
    },

    /// Delete the value at the path.
    ///
    /// No-op if the path doesn't exist.
    Delete {
        /// Target path.
        path: Path,
    },

    /// Append a value to an array at the path.
    ///
    /// Creates the array if it doesn't exist.
    /// Returns error if the target exists but is not an array.
    Append {
        /// Target path (must be an array or non-existent).
        path: Path,
        /// Value to append.
        value: Value,
    },

    /// Merge an object into the object at the path.
    ///
    /// Creates the object if it doesn't exist.
    /// Returns error if the target exists but is not an object.
    MergeObject {
        /// Target path (must be an object or non-existent).
        path: Path,
        /// Object to merge.
        value: Value,
    },

    /// Increment a numeric value at the path.
    ///
    /// Returns error if the target is not a number.
    Increment {
        /// Target path (must be a number).
        path: Path,
        /// Amount to increment by.
        amount: Number,
    },

    /// Decrement a numeric value at the path.
    ///
    /// Returns error if the target is not a number.
    Decrement {
        /// Target path (must be a number).
        path: Path,
        /// Amount to decrement by.
        amount: Number,
    },

    /// Insert a value at a specific index in an array.
    ///
    /// Shifts elements to the right.
    /// Returns error if index is out of bounds or target is not an array.
    Insert {
        /// Target path (must be an array).
        path: Path,
        /// Index to insert at.
        index: usize,
        /// Value to insert.
        value: Value,
    },

    /// Remove the first occurrence of a value from an array.
    ///
    /// No-op if the value is not found.
    /// Returns error if the target is not an array.
    Remove {
        /// Target path (must be an array).
        path: Path,
        /// Value to remove.
        value: Value,
    },
}

impl Op {
    // Convenience constructors

    /// Create a Set operation.
    #[inline]
    pub fn set(path: Path, value: impl Into<Value>) -> Self {
        Op::Set {
            path,
            value: value.into(),
        }
    }

    /// Create a Delete operation.
    #[inline]
    pub fn delete(path: Path) -> Self {
        Op::Delete { path }
    }

    /// Create an Append operation.
    #[inline]
    pub fn append(path: Path, value: impl Into<Value>) -> Self {
        Op::Append {
            path,
            value: value.into(),
        }
    }

    /// Create a MergeObject operation.
    #[inline]
    pub fn merge_object(path: Path, value: impl Into<Value>) -> Self {
        Op::MergeObject {
            path,
            value: value.into(),
        }
    }

    /// Create an Increment operation.
    #[inline]
    pub fn increment(path: Path, amount: impl Into<Number>) -> Self {
        Op::Increment {
            path,
            amount: amount.into(),
        }
    }

    /// Create a Decrement operation.
    #[inline]
    pub fn decrement(path: Path, amount: impl Into<Number>) -> Self {
        Op::Decrement {
            path,
            amount: amount.into(),
        }
    }

    /// Create an Insert operation.
    #[inline]
    pub fn insert(path: Path, index: usize, value: impl Into<Value>) -> Self {
        Op::Insert {
            path,
            index,
            value: value.into(),
        }
    }

    /// Create a Remove operation.
    #[inline]
    pub fn remove(path: Path, value: impl Into<Value>) -> Self {
        Op::Remove {
            path,
            value: value.into(),
        }
    }

    /// Get the path this operation targets.
    #[inline]
    pub fn path(&self) -> &Path {
        match self {
            Op::Set { path, .. } => path,
            Op::Delete { path } => path,
            Op::Append { path, .. } => path,
            Op::MergeObject { path, .. } => path,
            Op::Increment { path, .. } => path,
            Op::Decrement { path, .. } => path,
            Op::Insert { path, .. } => path,
            Op::Remove { path, .. } => path,
        }
    }

    /// Get a mutable reference to the path.
    #[inline]
    pub fn path_mut(&mut self) -> &mut Path {
        match self {
            Op::Set { path, .. } => path,
            Op::Delete { path } => path,
            Op::Append { path, .. } => path,
            Op::MergeObject { path, .. } => path,
            Op::Increment { path, .. } => path,
            Op::Decrement { path, .. } => path,
            Op::Insert { path, .. } => path,
            Op::Remove { path, .. } => path,
        }
    }

    /// Get the operation name.
    #[inline]
    pub fn name(&self) -> &'static str {
        match self {
            Op::Set { .. } => "set",
            Op::Delete { .. } => "delete",
            Op::Append { .. } => "append",
            Op::MergeObject { .. } => "merge_object",
            Op::Increment { .. } => "increment",
            Op::Decrement { .. } => "decrement",
            Op::Insert { .. } => "insert",
            Op::Remove { .. } => "remove",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::path;
    use serde_json::json;

    #[test]
    fn test_op_constructors() {
        let set = Op::set(path!("a"), json!(1));
        assert_eq!(set.name(), "set");
        assert_eq!(set.path(), &path!("a"));

        let del = Op::delete(path!("b"));
        assert_eq!(del.name(), "delete");

        let inc = Op::increment(path!("c"), 5i64);
        assert_eq!(inc.name(), "increment");
    }

    #[test]
    fn test_op_serde() {
        let op = Op::set(path!("users", 0, "name"), json!("Alice"));
        let json = serde_json::to_string(&op).unwrap();
        let parsed: Op = serde_json::from_str(&json).unwrap();
        assert_eq!(op, parsed);
    }

    #[test]
    fn test_number_conversions() {
        let n: Number = 42i64.into();
        assert!(n.is_int());
        assert_eq!(n.as_i64(), 42);

        let n: Number = 1.5f64.into();
        assert!(n.is_float());
        assert!((n.as_f64() - 1.5).abs() < 0.001);
    }
}
