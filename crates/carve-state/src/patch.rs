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
    /// This optimization:
    /// - Combines multiple `Set` operations to the same path (keeps last)
    /// - Removes `Set` followed by `Delete` on the same path (keeps Delete)
    /// - Combines consecutive `Increment`/`Decrement` on the same path
    /// - Combines consecutive `MergeObject` operations on the same path
    ///
    /// Note: Operations that affect parent/child paths or array operations
    /// (Append, Insert, Remove) are not optimized as they have complex interactions.
    ///
    /// # Examples
    ///
    /// ```
    /// use carve_state::{Patch, Op, path};
    /// use serde_json::json;
    ///
    /// let patch = Patch::new()
    ///     .with_op(Op::set(path!("name"), json!("Alice")))
    ///     .with_op(Op::set(path!("name"), json!("Bob")))  // overwrites Alice
    ///     .with_op(Op::increment(path!("count"), 1i64))
    ///     .with_op(Op::increment(path!("count"), 2i64)); // combine with above
    ///
    /// let canonical = patch.canonicalize();
    /// assert_eq!(canonical.len(), 2); // Set(name, Bob) + Increment(count, 3)
    /// ```
    pub fn canonicalize(&self) -> Patch {
        use crate::{Number, Path};
        use std::collections::HashMap;

        // Track the effective operation for each path
        // We use indices to preserve order for paths we haven't seen
        #[derive(Clone)]
        enum Effective {
            Set(Value),
            Delete,
            IncrementAmount(Number),
            MergeObject(serde_json::Map<String, Value>),
        }

        let mut path_effects: HashMap<Path, Effective> = HashMap::new();
        let mut path_order: Vec<Path> = Vec::new();
        let mut non_optimizable: Vec<Op> = Vec::new();

        for op in &self.ops {
            let path = op.path().clone();

            match op {
                Op::Set { value, .. } => {
                    if !path_effects.contains_key(&path) {
                        path_order.push(path.clone());
                    }
                    // Set always overwrites previous Set or IncrementAmount
                    path_effects.insert(path, Effective::Set(value.clone()));
                }
                Op::Delete { .. } => {
                    if !path_effects.contains_key(&path) {
                        path_order.push(path.clone());
                    }
                    // Delete overwrites any previous operation on this exact path
                    path_effects.insert(path, Effective::Delete);
                }
                Op::Increment { amount, .. } => {
                    match path_effects.get(&path) {
                        Some(Effective::IncrementAmount(existing)) => {
                            // Combine increments
                            let combined = combine_numbers(existing, amount, true);
                            path_effects.insert(path, Effective::IncrementAmount(combined));
                        }
                        Some(Effective::Set(_)) | Some(Effective::Delete) => {
                            // Can't combine with Set or Delete - emit as separate op
                            non_optimizable.push(op.clone());
                        }
                        Some(Effective::MergeObject(_)) => {
                            non_optimizable.push(op.clone());
                        }
                        None => {
                            path_order.push(path.clone());
                            path_effects.insert(path, Effective::IncrementAmount(amount.clone()));
                        }
                    }
                }
                Op::Decrement { amount, .. } => {
                    match path_effects.get(&path) {
                        Some(Effective::IncrementAmount(existing)) => {
                            // Combine: existing - amount
                            let combined = combine_numbers(existing, amount, false);
                            path_effects.insert(path, Effective::IncrementAmount(combined));
                        }
                        Some(Effective::Set(_)) | Some(Effective::Delete) => {
                            non_optimizable.push(op.clone());
                        }
                        Some(Effective::MergeObject(_)) => {
                            non_optimizable.push(op.clone());
                        }
                        None => {
                            path_order.push(path.clone());
                            // Decrement by X is same as Increment by -X
                            let neg = negate_number(amount);
                            path_effects.insert(path, Effective::IncrementAmount(neg));
                        }
                    }
                }
                Op::MergeObject { value, .. } => {
                    if let Some(obj) = value.as_object() {
                        match path_effects.get(&path) {
                            Some(Effective::MergeObject(existing)) => {
                                // Combine merge objects
                                let mut combined = existing.clone();
                                for (k, v) in obj {
                                    combined.insert(k.clone(), v.clone());
                                }
                                path_effects.insert(path, Effective::MergeObject(combined));
                            }
                            Some(Effective::Set(_)) | Some(Effective::Delete) => {
                                non_optimizable.push(op.clone());
                            }
                            Some(Effective::IncrementAmount(_)) => {
                                non_optimizable.push(op.clone());
                            }
                            None => {
                                path_order.push(path.clone());
                                path_effects.insert(path, Effective::MergeObject(obj.clone()));
                            }
                        }
                    } else {
                        non_optimizable.push(op.clone());
                    }
                }
                // Array operations are not optimized - they have complex order-dependent behavior
                Op::Append { .. } | Op::Insert { .. } | Op::Remove { .. } => {
                    non_optimizable.push(op.clone());
                }
            }
        }

        // Build the canonicalized patch in order
        let mut result = Patch::new();

        for path in path_order {
            if let Some(effect) = path_effects.remove(&path) {
                match effect {
                    Effective::Set(value) => {
                        result.push(Op::Set { path, value });
                    }
                    Effective::Delete => {
                        result.push(Op::Delete { path });
                    }
                    Effective::IncrementAmount(amount) => {
                        // If amount is negative, emit Decrement; otherwise Increment
                        // But first check if it's zero - skip if so
                        if is_zero(&amount) {
                            continue;
                        }
                        if is_negative(&amount) {
                            result.push(Op::Decrement {
                                path,
                                amount: negate_number(&amount),
                            });
                        } else {
                            result.push(Op::Increment { path, amount });
                        }
                    }
                    Effective::MergeObject(map) => {
                        result.push(Op::MergeObject {
                            path,
                            value: Value::Object(map),
                        });
                    }
                }
            }
        }

        // Add non-optimizable operations at the end (preserving their order)
        for op in non_optimizable {
            result.push(op);
        }

        result
    }
}

// Helper functions for Number arithmetic
use serde_json::Value;

fn combine_numbers(a: &crate::Number, b: &crate::Number, add: bool) -> crate::Number {
    match (a, b) {
        (crate::Number::Int(x), crate::Number::Int(y)) => {
            if add {
                crate::Number::Int(x + y)
            } else {
                crate::Number::Int(x - y)
            }
        }
        _ => {
            let x = a.as_f64();
            let y = b.as_f64();
            if add {
                crate::Number::Float(x + y)
            } else {
                crate::Number::Float(x - y)
            }
        }
    }
}

fn negate_number(n: &crate::Number) -> crate::Number {
    match n {
        crate::Number::Int(i) => crate::Number::Int(-i),
        crate::Number::Float(f) => crate::Number::Float(-f),
    }
}

fn is_zero(n: &crate::Number) -> bool {
    match n {
        crate::Number::Int(i) => *i == 0,
        crate::Number::Float(f) => f.abs() < f64::EPSILON,
    }
}

fn is_negative(n: &crate::Number) -> bool {
    match n {
        crate::Number::Int(i) => *i < 0,
        crate::Number::Float(f) => *f < 0.0,
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
        assert_eq!(
            canonical.ops()[0],
            Op::set(path!("name"), json!("Charlie"))
        );
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
    fn test_canonicalize_combine_increments() {
        let patch = Patch::new()
            .with_op(Op::increment(path!("count"), 1i64))
            .with_op(Op::increment(path!("count"), 2i64))
            .with_op(Op::increment(path!("count"), 3i64));

        let canonical = patch.canonicalize();
        assert_eq!(canonical.len(), 1);
        assert_eq!(
            canonical.ops()[0],
            Op::increment(path!("count"), 6i64)
        );
    }

    #[test]
    fn test_canonicalize_combine_increment_decrement() {
        let patch = Patch::new()
            .with_op(Op::increment(path!("count"), 10i64))
            .with_op(Op::decrement(path!("count"), 3i64));

        let canonical = patch.canonicalize();
        assert_eq!(canonical.len(), 1);
        assert_eq!(
            canonical.ops()[0],
            Op::increment(path!("count"), 7i64)
        );
    }

    #[test]
    fn test_canonicalize_decrement_to_negative() {
        let patch = Patch::new()
            .with_op(Op::increment(path!("count"), 5i64))
            .with_op(Op::decrement(path!("count"), 10i64));

        let canonical = patch.canonicalize();
        assert_eq!(canonical.len(), 1);
        // Result is -5, so should be emitted as Decrement(5)
        assert_eq!(
            canonical.ops()[0],
            Op::decrement(path!("count"), 5i64)
        );
    }

    #[test]
    fn test_canonicalize_zero_increment_removed() {
        let patch = Patch::new()
            .with_op(Op::increment(path!("count"), 5i64))
            .with_op(Op::decrement(path!("count"), 5i64));

        let canonical = patch.canonicalize();
        // Net zero - operation should be removed
        assert!(canonical.is_empty());
    }

    #[test]
    fn test_canonicalize_combine_merge_objects() {
        let patch = Patch::new()
            .with_op(Op::merge_object(path!("user"), json!({"name": "Alice"})))
            .with_op(Op::merge_object(path!("user"), json!({"age": 30})))
            .with_op(Op::merge_object(path!("user"), json!({"email": "alice@example.com"})));

        let canonical = patch.canonicalize();
        assert_eq!(canonical.len(), 1);
        if let Op::MergeObject { value, .. } = &canonical.ops()[0] {
            assert_eq!(value["name"], "Alice");
            assert_eq!(value["age"], 30);
            assert_eq!(value["email"], "alice@example.com");
        } else {
            panic!("Expected MergeObject");
        }
    }

    #[test]
    fn test_canonicalize_preserves_different_paths() {
        let patch = Patch::new()
            .with_op(Op::set(path!("a"), json!(1)))
            .with_op(Op::set(path!("b"), json!(2)))
            .with_op(Op::set(path!("a"), json!(10)));

        let canonical = patch.canonicalize();
        assert_eq!(canonical.len(), 2);
        // Order preserved: a first (with last value), then b
        assert_eq!(canonical.ops()[0], Op::set(path!("a"), json!(10)));
        assert_eq!(canonical.ops()[1], Op::set(path!("b"), json!(2)));
    }

    #[test]
    fn test_canonicalize_array_ops_not_optimized() {
        let patch = Patch::new()
            .with_op(Op::append(path!("items"), json!(1)))
            .with_op(Op::append(path!("items"), json!(2)))
            .with_op(Op::insert(path!("items"), 0, json!(0)));

        let canonical = patch.canonicalize();
        // Array ops are passed through unchanged
        assert_eq!(canonical.len(), 3);
    }

    #[test]
    fn test_canonicalize_empty_patch() {
        let patch = Patch::new();
        let canonical = patch.canonicalize();
        assert!(canonical.is_empty());
    }

    #[test]
    fn test_canonicalize_float_increments() {
        let patch = Patch::new()
            .with_op(Op::increment(path!("value"), 1.5f64))
            .with_op(Op::increment(path!("value"), 2.5f64));

        let canonical = patch.canonicalize();
        assert_eq!(canonical.len(), 1);
        if let Op::Increment { amount, .. } = &canonical.ops()[0] {
            assert!((amount.as_f64() - 4.0).abs() < f64::EPSILON);
        } else {
            panic!("Expected Increment");
        }
    }
}
