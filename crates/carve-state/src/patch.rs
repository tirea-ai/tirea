//! Patch containers for grouping operations.
//!
//! A `Patch` is a collection of operations that can be applied atomically
//! to a JSON document. `TrackedPatch` adds metadata for debugging and auditing.

use crate::Op;
use serde::{Deserialize, Serialize};

/// A collection of operations to apply atomically.
///
/// Patches are the primary way to describe changes to a document.
/// Operations are applied in order.
///
/// # Examples
///
/// ```
/// use carve_state::{Patch, Op, path};
/// use serde_json::json;
///
/// let patch = Patch::new()
///     .with_op(Op::set(path!("name"), json!("Alice")))
///     .with_op(Op::set(path!("age"), json!(30)));
///
/// assert_eq!(patch.len(), 2);
/// ```
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Patch {
    /// The operations in this patch.
    ops: Vec<Op>,
}

impl Patch {
    /// Create an empty patch.
    #[inline]
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a patch with the given operations.
    #[inline]
    pub fn with_ops(ops: Vec<Op>) -> Self {
        Self { ops }
    }

    /// Add an operation to this patch (builder pattern).
    #[inline]
    pub fn with_op(mut self, op: Op) -> Self {
        self.ops.push(op);
        self
    }

    /// Push an operation onto this patch.
    #[inline]
    pub fn push(&mut self, op: Op) {
        self.ops.push(op);
    }

    /// Get the operations in this patch.
    #[inline]
    pub fn ops(&self) -> &[Op] {
        &self.ops
    }

    /// Get mutable access to the operations.
    #[inline]
    pub fn ops_mut(&mut self) -> &mut Vec<Op> {
        &mut self.ops
    }

    /// Consume this patch and return the operations.
    #[inline]
    pub fn into_ops(self) -> Vec<Op> {
        self.ops
    }

    /// Check if this patch is empty.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.ops.is_empty()
    }

    /// Get the number of operations in this patch.
    #[inline]
    pub fn len(&self) -> usize {
        self.ops.len()
    }

    /// Extend this patch with operations from another patch.
    #[inline]
    pub fn extend(&mut self, other: Patch) {
        self.ops.extend(other.ops);
    }

    /// Merge another patch into this one (alias for extend).
    #[inline]
    pub fn merge(&mut self, other: Patch) {
        self.extend(other);
    }

    /// Clear all operations from this patch.
    #[inline]
    pub fn clear(&mut self) {
        self.ops.clear();
    }

    /// Iterate over the operations.
    #[inline]
    pub fn iter(&self) -> impl Iterator<Item = &Op> {
        self.ops.iter()
    }
}

impl FromIterator<Op> for Patch {
    fn from_iter<I: IntoIterator<Item = Op>>(iter: I) -> Self {
        Self {
            ops: iter.into_iter().collect(),
        }
    }
}

impl IntoIterator for Patch {
    type Item = Op;
    type IntoIter = std::vec::IntoIter<Op>;

    fn into_iter(self) -> Self::IntoIter {
        self.ops.into_iter()
    }
}

impl<'a> IntoIterator for &'a Patch {
    type Item = &'a Op;
    type IntoIter = std::slice::Iter<'a, Op>;

    fn into_iter(self) -> Self::IntoIter {
        self.ops.iter()
    }
}

impl Extend<Op> for Patch {
    fn extend<I: IntoIterator<Item = Op>>(&mut self, iter: I) {
        self.ops.extend(iter);
    }
}

/// A patch with tracking metadata.
///
/// `TrackedPatch` wraps a `Patch` with additional information useful for
/// debugging, auditing, and conflict detection.
///
/// # Examples
///
/// ```
/// use carve_state::{Patch, TrackedPatch, Op, path};
/// use serde_json::json;
///
/// let patch = Patch::new()
///     .with_op(Op::set(path!("counter"), json!(1)));
///
/// let tracked = TrackedPatch::new(patch)
///     .with_id("patch-001")
///     .with_source("user-service");
/// ```
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TrackedPatch {
    /// The underlying patch.
    pub patch: Patch,

    /// Unique identifier for this patch.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,

    /// Timestamp when this patch was created (Unix epoch millis).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<u64>,

    /// Source/origin of this patch (e.g., service name, user ID).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,

    /// Optional description of what this patch does.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

impl TrackedPatch {
    /// Create a new tracked patch.
    #[inline]
    pub fn new(patch: Patch) -> Self {
        Self {
            patch,
            id: None,
            timestamp: None,
            source: None,
            description: None,
        }
    }

    /// Set the patch ID (builder pattern).
    #[inline]
    pub fn with_id(mut self, id: impl Into<String>) -> Self {
        self.id = Some(id.into());
        self
    }

    /// Set the timestamp (builder pattern).
    #[inline]
    pub fn with_timestamp(mut self, ts: u64) -> Self {
        self.timestamp = Some(ts);
        self
    }

    /// Set the source (builder pattern).
    #[inline]
    pub fn with_source(mut self, source: impl Into<String>) -> Self {
        self.source = Some(source.into());
        self
    }

    /// Set the description (builder pattern).
    #[inline]
    pub fn with_description(mut self, desc: impl Into<String>) -> Self {
        self.description = Some(desc.into());
        self
    }

    /// Get the underlying patch.
    #[inline]
    pub fn patch(&self) -> &Patch {
        &self.patch
    }

    /// Consume and return the underlying patch.
    #[inline]
    pub fn into_patch(self) -> Patch {
        self.patch
    }
}

impl From<Patch> for TrackedPatch {
    fn from(patch: Patch) -> Self {
        TrackedPatch::new(patch)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::path;
    use serde_json::json;

    #[test]
    fn test_patch_builder() {
        let patch = Patch::new()
            .with_op(Op::set(path!("a"), json!(1)))
            .with_op(Op::set(path!("b"), json!(2)));

        assert_eq!(patch.len(), 2);
    }

    #[test]
    fn test_patch_extend() {
        let mut p1 = Patch::new().with_op(Op::set(path!("a"), json!(1)));
        let p2 = Patch::new().with_op(Op::set(path!("b"), json!(2)));

        p1.extend(p2);
        assert_eq!(p1.len(), 2);
    }

    #[test]
    fn test_patch_serde() {
        let patch = Patch::new()
            .with_op(Op::set(path!("name"), json!("test")))
            .with_op(Op::increment(path!("count"), 1i64));

        let json = serde_json::to_string(&patch).unwrap();
        let parsed: Patch = serde_json::from_str(&json).unwrap();
        assert_eq!(patch, parsed);
    }

    #[test]
    fn test_tracked_patch() {
        let patch = Patch::new().with_op(Op::set(path!("x"), json!(42)));

        let tracked = TrackedPatch::new(patch)
            .with_id("test-001")
            .with_source("test")
            .with_timestamp(1234567890);

        assert_eq!(tracked.id, Some("test-001".into()));
        assert_eq!(tracked.source, Some("test".into()));
        assert_eq!(tracked.timestamp, Some(1234567890));
    }
}
