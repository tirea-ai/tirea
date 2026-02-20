//! Generic JSON writer for building patches.
//!
//! `JsonWriter` provides a low-level API for building patches dynamically.
//! For type-safe access, use derived `State` types instead.

use crate::{Number, Op, Patch, Path};
use serde_json::Value;

/// Internal trait for writer operations.
///
/// This trait is implemented by all generated writers and `JsonWriter`.
/// It provides a common interface for accessing and manipulating operations.
#[doc(hidden)]
pub trait WriterOps {
    /// Get the accumulated operations.
    fn ops(&self) -> &[Op];

    /// Get mutable access to the operations.
    fn ops_mut(&mut self) -> &mut Vec<Op>;

    /// Take all operations, leaving the writer empty.
    fn take_ops(&mut self) -> Vec<Op>;

    /// Consume and build a patch.
    fn into_patch(self) -> Patch;
}

/// A generic writer for building patches on arbitrary JSON structures.
///
/// `JsonWriter` is an escape hatch for cases where you need to work with
/// dynamic or unknown JSON structures. For typed access, prefer using
/// derived `State` types.
///
/// # Examples
///
/// ```
/// use tirea_state::{JsonWriter, path};
/// use serde_json::json;
///
/// let mut w = JsonWriter::new();
/// w.set(path!("name"), json!("Alice"));
/// w.set(path!("age"), json!(30));
/// w.append(path!("tags"), json!("admin"));
///
/// let patch = w.build();
/// assert_eq!(patch.len(), 3);
/// ```
#[derive(Debug, Clone)]
pub struct JsonWriter {
    base: Path,
    ops: Vec<Op>,
}

impl JsonWriter {
    /// Create a new writer at the document root.
    #[inline]
    pub fn new() -> Self {
        Self {
            base: Path::root(),
            ops: Vec::new(),
        }
    }

    /// Create a new writer at the specified base path.
    #[inline]
    pub fn at(base: Path) -> Self {
        Self {
            base,
            ops: Vec::new(),
        }
    }

    /// Get the base path of this writer.
    #[inline]
    pub fn base(&self) -> &Path {
        &self.base
    }

    /// Create a nested writer at a relative path.
    ///
    /// The nested writer will have its own operations vector.
    /// Use `merge` to combine its operations back into the parent.
    #[inline]
    pub fn nested(&self, path: Path) -> Self {
        Self {
            base: self.base.join(&path),
            ops: Vec::new(),
        }
    }

    /// Compute the full path by joining base with relative path.
    #[inline]
    fn full_path(&self, path: Path) -> Path {
        if path.is_empty() {
            self.base.clone()
        } else {
            self.base.join(&path)
        }
    }

    /// Set a value at the specified path.
    #[inline]
    pub fn set(&mut self, path: Path, value: impl Into<Value>) -> &mut Self {
        self.ops.push(Op::Set {
            path: self.full_path(path),
            value: value.into(),
        });
        self
    }

    /// Set a value at the base path.
    #[inline]
    pub fn set_root(&mut self, value: impl Into<Value>) -> &mut Self {
        self.ops.push(Op::Set {
            path: self.base.clone(),
            value: value.into(),
        });
        self
    }

    /// Delete the value at the specified path.
    #[inline]
    pub fn delete(&mut self, path: Path) -> &mut Self {
        self.ops.push(Op::Delete {
            path: self.full_path(path),
        });
        self
    }

    /// Append a value to an array at the specified path.
    #[inline]
    pub fn append(&mut self, path: Path, value: impl Into<Value>) -> &mut Self {
        self.ops.push(Op::Append {
            path: self.full_path(path),
            value: value.into(),
        });
        self
    }

    /// Merge an object into the object at the specified path.
    #[inline]
    pub fn merge_object(&mut self, path: Path, value: impl Into<Value>) -> &mut Self {
        self.ops.push(Op::MergeObject {
            path: self.full_path(path),
            value: value.into(),
        });
        self
    }

    /// Increment a numeric value at the specified path.
    #[inline]
    pub fn increment(&mut self, path: Path, amount: impl Into<Number>) -> &mut Self {
        self.ops.push(Op::Increment {
            path: self.full_path(path),
            amount: amount.into(),
        });
        self
    }

    /// Decrement a numeric value at the specified path.
    #[inline]
    pub fn decrement(&mut self, path: Path, amount: impl Into<Number>) -> &mut Self {
        self.ops.push(Op::Decrement {
            path: self.full_path(path),
            amount: amount.into(),
        });
        self
    }

    /// Insert a value at a specific index in an array.
    #[inline]
    pub fn insert(&mut self, path: Path, index: usize, value: impl Into<Value>) -> &mut Self {
        self.ops.push(Op::Insert {
            path: self.full_path(path),
            index,
            value: value.into(),
        });
        self
    }

    /// Remove the first occurrence of a value from an array.
    #[inline]
    pub fn remove(&mut self, path: Path, value: impl Into<Value>) -> &mut Self {
        self.ops.push(Op::Remove {
            path: self.full_path(path),
            value: value.into(),
        });
        self
    }

    /// Merge operations from another writer into this one.
    #[inline]
    pub fn merge<W: WriterOps>(&mut self, mut other: W) -> &mut Self {
        self.ops.extend(other.take_ops());
        self
    }

    /// Consume this writer and build a patch.
    #[inline]
    pub fn build(self) -> Patch {
        Patch::with_ops(self.ops)
    }

    /// Check if this writer has any operations.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.ops.is_empty()
    }

    /// Get the number of operations.
    #[inline]
    pub fn len(&self) -> usize {
        self.ops.len()
    }

    /// Clear all operations.
    #[inline]
    pub fn clear(&mut self) {
        self.ops.clear();
    }
}

impl Default for JsonWriter {
    fn default() -> Self {
        Self::new()
    }
}

impl WriterOps for JsonWriter {
    fn ops(&self) -> &[Op] {
        &self.ops
    }

    fn ops_mut(&mut self) -> &mut Vec<Op> {
        &mut self.ops
    }

    fn take_ops(&mut self) -> Vec<Op> {
        std::mem::take(&mut self.ops)
    }

    fn into_patch(self) -> Patch {
        self.build()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::path;
    use serde_json::json;

    #[test]
    fn test_json_writer_basic() {
        let mut w = JsonWriter::new();
        w.set(path!("name"), json!("Alice"));
        w.set(path!("age"), json!(30));

        let patch = w.build();
        assert_eq!(patch.len(), 2);
    }

    #[test]
    fn test_json_writer_with_base() {
        let mut w = JsonWriter::at(path!("user"));
        w.set(path!("name"), json!("Bob"));

        let patch = w.build();
        assert_eq!(patch.ops()[0].path(), &path!("user", "name"));
    }

    #[test]
    fn test_json_writer_nested() {
        let w = JsonWriter::at(path!("data"));
        let nested = w.nested(path!("items"));
        assert_eq!(nested.base(), &path!("data", "items"));
    }

    #[test]
    fn test_json_writer_merge() {
        let mut w1 = JsonWriter::new();
        w1.set(path!("a"), json!(1));

        let mut w2 = JsonWriter::new();
        w2.set(path!("b"), json!(2));

        w1.merge(w2);
        assert_eq!(w1.len(), 2);
    }

    #[test]
    fn test_json_writer_all_ops() {
        let mut w = JsonWriter::new();

        w.set(path!("x"), json!(1));
        w.delete(path!("y"));
        w.append(path!("arr"), json!(1));
        w.merge_object(path!("obj"), json!({"a": 1}));
        w.increment(path!("count"), 1i64);
        w.decrement(path!("score"), 5i64);
        w.insert(path!("list"), 0, json!("first"));
        w.remove(path!("tags"), json!("old"));

        assert_eq!(w.len(), 8);
    }
}
