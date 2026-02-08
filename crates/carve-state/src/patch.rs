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

    /// Canonicalize the patch by removing redundant operations.
    ///
    /// This is a **safe, semantics-preserving** optimization that:
    /// - Removes earlier `Set` operations when a later `Set` targets the same path
    /// - Removes earlier `Set` operations when a later `Delete` targets the same path
    /// - Removes earlier `Delete` operations when a later `Set` or `Delete` targets the same path
    ///
    /// **Important**: This function only *removes* redundant operations. It never reorders
    /// the remaining operations. The output is always a subsequence of the input in the
    /// same relative order.
    ///
    /// Operations that are **not** optimized (kept as-is):
    /// - `Increment`, `Decrement`, `MergeObject`: These have cumulative effects and
    ///   cannot be safely deduplicated without semantic analysis
    /// - `Append`, `Insert`, `Remove`: Array operations are order-sensitive
    ///
    /// # Examples
    ///
    /// ```
    /// use carve_state::{Patch, Op, path};
    /// use serde_json::json;
    ///
    /// let patch = Patch::new()
    ///     .with_op(Op::set(path!("name"), json!("Alice")))
    ///     .with_op(Op::set(path!("name"), json!("Bob")));  // overwrites Alice
    ///
    /// let canonical = patch.canonicalize();
    /// assert_eq!(canonical.len(), 1);
    /// assert_eq!(canonical.ops()[0], Op::set(path!("name"), json!("Bob")));
    /// ```
    pub fn canonicalize(&self) -> Patch {
        use crate::Path;
        use std::collections::HashSet;

        if self.ops.is_empty() {
            return Patch::new();
        }

        // Backward scan algorithm for semantic-preserving canonicalization
        //
        // Key insight: A later Set/Delete at path P makes earlier operations at P
        // or any child of P irrelevant, because Set/Delete overwrites everything.
        //
        // Algorithm:
        // 1. Scan operations backward (from end to start)
        // 2. Maintain a set of "covered" paths
        // 3. For each operation:
        //    - If its path is covered by any later Set/Delete → skip (redundant)
        //    - Otherwise → keep it
        //    - If it's a Set/Delete → mark its path as covered
        // 4. Reverse the result to restore original order

        let mut covered_paths: HashSet<Path> = HashSet::new();
        let mut kept_ops: Vec<Op> = Vec::new();

        // Scan backward
        for op in self.ops.iter().rev() {
            let op_path = op.path();

            // Check if this operation is covered by a later Set/Delete
            let is_covered = covered_paths.iter().any(|covered| {
                // Case 1: Exact match - later op at same path覆盖s this one
                if covered == op_path {
                    return true;
                }
                // Case 2: covered is prefix of op_path - parent Set覆盖s child op
                // e.g., Set(user)覆盖s Increment(user.age)
                if covered.is_prefix_of(op_path) {
                    return true;
                }
                false
            });

            if is_covered {
                // This operation is redundant, skip it
                continue;
            }

            // Keep this operation
            kept_ops.push(op.clone());

            // If this is a Set/Delete, mark its path (and implicitly all children) as covered
            match op {
                Op::Set { path, .. } | Op::Delete { path } => {
                    // Also remove any covered paths that are children of this path
                    // because Set(user) makes Set(user.name) coverage redundant
                    covered_paths.retain(|p| !path.is_prefix_of(p));
                    covered_paths.insert(path.clone());
                }
                _ => {
                    // Non-Set/Delete operations don't create coverage
                    // They can still be覆盖d by later Sets, but don't覆盖 others
                }
            }
        }

        // Reverse to restore original order
        kept_ops.reverse();

        Patch::with_ops(kept_ops)
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

    #[test]
    fn test_canonicalize_multiple_sets() {
        let patch = Patch::new()
            .with_op(Op::set(path!("name"), json!("Alice")))
            .with_op(Op::set(path!("name"), json!("Bob")))
            .with_op(Op::set(path!("name"), json!("Charlie")));

        let canonical = patch.canonicalize();
        assert_eq!(canonical.len(), 1);
        assert_eq!(canonical.ops()[0], Op::set(path!("name"), json!("Charlie")));
    }

    #[test]
    fn test_canonicalize_set_then_delete() {
        let patch = Patch::new()
            .with_op(Op::set(path!("x"), json!(42)))
            .with_op(Op::delete(path!("x")));

        let canonical = patch.canonicalize();
        assert_eq!(canonical.len(), 1);
        assert_eq!(canonical.ops()[0], Op::delete(path!("x")));
    }

    #[test]
    fn test_canonicalize_increments_not_combined() {
        // Safe canonicalize does NOT combine increments (they have cumulative effects)
        let patch = Patch::new()
            .with_op(Op::increment(path!("count"), 1i64))
            .with_op(Op::increment(path!("count"), 2i64))
            .with_op(Op::increment(path!("count"), 3i64));

        let canonical = patch.canonicalize();
        // All increments are kept
        assert_eq!(canonical.len(), 3);

        // Verify semantics are preserved
        let doc = json!({"count": 0});
        let result_original = crate::apply_patch(&doc, &patch).unwrap();
        let result_canonical = crate::apply_patch(&doc, &canonical).unwrap();
        assert_eq!(result_original, result_canonical);
    }

    #[test]
    fn test_canonicalize_increment_decrement_not_combined() {
        // Safe canonicalize does NOT combine increment/decrement
        let patch = Patch::new()
            .with_op(Op::increment(path!("count"), 10i64))
            .with_op(Op::decrement(path!("count"), 3i64));

        let canonical = patch.canonicalize();
        assert_eq!(canonical.len(), 2);

        // Verify semantics
        let doc = json!({"count": 0});
        let result_original = crate::apply_patch(&doc, &patch).unwrap();
        let result_canonical = crate::apply_patch(&doc, &canonical).unwrap();
        assert_eq!(result_original, result_canonical);
    }

    #[test]
    fn test_canonicalize_merge_objects_not_combined() {
        // Safe canonicalize does NOT combine merge objects
        let patch = Patch::new()
            .with_op(Op::merge_object(path!("user"), json!({"name": "Alice"})))
            .with_op(Op::merge_object(path!("user"), json!({"age": 30})));

        let canonical = patch.canonicalize();
        assert_eq!(canonical.len(), 2);

        // Verify semantics
        let doc = json!({"user": {}});
        let result_original = crate::apply_patch(&doc, &patch).unwrap();
        let result_canonical = crate::apply_patch(&doc, &canonical).unwrap();
        assert_eq!(result_original, result_canonical);
    }

    #[test]
    fn test_canonicalize_preserves_different_paths() {
        // Input: set(a,1), set(b,2), set(a,10)
        // Canonicalize removes set(a,1) because set(a,10) is the last for path "a"
        // Result is a subsequence: set(b,2), set(a,10)
        //
        // IMPORTANT: This is NOT reordering. The output preserves the relative
        // order of the remaining operations. b was at index 1, a's last occurrence
        // was at index 2, so the output order is b -> a.
        let patch = Patch::new()
            .with_op(Op::set(path!("a"), json!(1))) // index 0 - REMOVED (not last for "a")
            .with_op(Op::set(path!("b"), json!(2))) // index 1 - KEPT (last for "b")
            .with_op(Op::set(path!("a"), json!(10))); // index 2 - KEPT (last for "a")

        let canonical = patch.canonicalize();
        assert_eq!(canonical.len(), 2);
        // Output is a subsequence in original order: ops at index 1, then index 2
        assert_eq!(canonical.ops()[0], Op::set(path!("b"), json!(2)));
        assert_eq!(canonical.ops()[1], Op::set(path!("a"), json!(10)));
    }

    #[test]
    fn test_canonicalize_array_ops_preserve_order() {
        let patch = Patch::new()
            .with_op(Op::append(path!("items"), json!(1)))
            .with_op(Op::set(path!("name"), json!("test")))
            .with_op(Op::append(path!("items"), json!(2)));

        let canonical = patch.canonicalize();
        // Array ops stay in their original positions
        assert_eq!(canonical.len(), 3);
        assert_eq!(canonical.ops()[0], Op::append(path!("items"), json!(1)));
        assert_eq!(canonical.ops()[1], Op::set(path!("name"), json!("test")));
        assert_eq!(canonical.ops()[2], Op::append(path!("items"), json!(2)));
    }

    #[test]
    fn test_canonicalize_preserves_operation_order() {
        // This test verifies that canonicalize doesn't reorder operations
        let patch = Patch::new()
            .with_op(Op::set(path!("user"), json!({})))
            .with_op(Op::append(path!("items"), json!(1)))
            .with_op(Op::set(path!("user", "name"), json!("Alice")));

        let canonical = patch.canonicalize();
        // Order must be preserved: Set(user) -> Append(items) -> Set(user.name)
        assert_eq!(canonical.len(), 3);
        assert_eq!(canonical.ops()[0], Op::set(path!("user"), json!({})));
        assert_eq!(canonical.ops()[1], Op::append(path!("items"), json!(1)));
        assert_eq!(
            canonical.ops()[2],
            Op::set(path!("user", "name"), json!("Alice"))
        );
    }

    #[test]
    fn test_canonicalize_interleaved_increment_set() {
        // Increment, Set, Increment 交错序列
        // 正确行为：Set 会重置值，后续的 Increment 应该保留
        let patch = Patch::new()
            .with_op(Op::increment(path!("x"), 1i64))
            .with_op(Op::set(path!("x"), json!(0)))
            .with_op(Op::increment(path!("x"), 2i64));

        let canonical = patch.canonicalize();

        // 验证语义一致性
        let doc = json!({"x": 100});
        let result_original = crate::apply_patch(&doc, &patch).unwrap();
        let result_canonical = crate::apply_patch(&doc, &canonical).unwrap();

        assert_eq!(
            result_original, result_canonical,
            "canonicalize changed semantics! original={}, canonical={}",
            result_original["x"], result_canonical["x"]
        );
    }

    #[test]
    fn test_canonicalize_interleaved_merge_set() {
        // MergeObject, Set, MergeObject 交错序列
        let patch = Patch::new()
            .with_op(Op::merge_object(path!("user"), json!({"a": 1})))
            .with_op(Op::set(path!("user"), json!({})))
            .with_op(Op::merge_object(path!("user"), json!({"b": 2})));

        let canonical = patch.canonicalize();

        let doc = json!({"user": {"existing": true}});
        let result_original = crate::apply_patch(&doc, &patch).unwrap();
        let result_canonical = crate::apply_patch(&doc, &canonical).unwrap();

        assert_eq!(
            result_original, result_canonical,
            "canonicalize changed semantics!"
        );
    }

    #[test]
    fn test_canonicalize_empty_patch() {
        let patch = Patch::new();
        let canonical = patch.canonicalize();
        assert!(canonical.is_empty());
    }

    #[test]
    fn test_canonicalize_semantics_preserved() {
        // Comprehensive test: canonicalize must never change semantics
        let patch = Patch::new()
            .with_op(Op::increment(path!("value"), 1.5f64))
            .with_op(Op::increment(path!("value"), 2.5f64));

        let canonical = patch.canonicalize();
        // Increments are not combined in safe mode
        assert_eq!(canonical.len(), 2);

        // But semantics must be identical
        let doc = json!({"value": 0.0});
        let result_original = crate::apply_patch(&doc, &patch).unwrap();
        let result_canonical = crate::apply_patch(&doc, &canonical).unwrap();
        assert_eq!(result_original, result_canonical);
    }

    #[test]
    fn test_canonicalize_set_increment_set_correct_behavior() {
        // Problem 1 from code review: Set -> Increment -> Set should remove BOTH
        // Set(1) and Increment because the final Set(2) overwrites everything.
        //
        // Original: set(x, 1) -> increment(x, +1) -> set(x, 2)
        // Correct semantic: x = 2 (final set wins, intermediate ops irrelevant)
        //
        // Correct canonicalize should produce: set(x, 2)
        // Only the final Set matters

        let patch = Patch::new()
            .with_op(Op::set(path!("x"), json!(1)))
            .with_op(Op::increment(path!("x"), 1i64))
            .with_op(Op::set(path!("x"), json!(2)));

        let canonical = patch.canonicalize();

        // Test with various initial docs
        let docs = vec![
            json!({}),          // x doesn't exist
            json!({"x": 100}),  // x has different value
            json!({"x": null}), // x is null
        ];

        for doc in docs {
            let result_original = crate::apply_patch(&doc, &patch).unwrap();
            let result_canonical = crate::apply_patch(&doc, &canonical).unwrap();

            assert_eq!(
                result_original, result_canonical,
                "canonicalize changed semantics for doc: {}",
                doc
            );

            // Both should result in x = 2
            assert_eq!(result_original["x"], json!(2));
        }

        // Canonical should be just: set(x, 2)
        // The final Set should have covered both previous Set and Increment
        assert_eq!(
            canonical.len(),
            1,
            "Expected 1 op after canonicalization, got: {:?}",
            canonical.ops()
        );

        // Verify it's a Set operation for path "x" with value 2
        match &canonical.ops()[0] {
            Op::Set { path, value } => {
                assert_eq!(path, &path!("x"));
                assert_eq!(value, &json!(2));
            }
            _ => panic!("Expected Set operation, got: {:?}", canonical.ops()[0]),
        }
    }
}
