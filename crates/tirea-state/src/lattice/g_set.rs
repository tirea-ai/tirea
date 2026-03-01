use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use super::Lattice;

/// A grow-only set (G-Set). Elements can be added but never removed.
///
/// Merge semantics: set union.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct GSet<T: Ord>(BTreeSet<T>);

impl<T: Ord> GSet<T> {
    /// Create an empty set.
    pub fn new() -> Self {
        Self(BTreeSet::new())
    }

    /// Insert an element. Returns `true` if the element was newly inserted.
    pub fn insert(&mut self, value: T) -> bool {
        self.0.insert(value)
    }

    /// Returns `true` if the set contains the given element.
    pub fn contains(&self, value: &T) -> bool {
        self.0.contains(value)
    }

    /// Returns the number of elements.
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Returns `true` if the set is empty.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Returns an iterator over the elements in sorted order.
    pub fn iter(&self) -> std::collections::btree_set::Iter<'_, T> {
        self.0.iter()
    }
}

impl<T: Ord> Default for GSet<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T: Ord> IntoIterator for GSet<T> {
    type Item = T;
    type IntoIter = std::collections::btree_set::IntoIter<T>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.into_iter()
    }
}

impl<'a, T: Ord> IntoIterator for &'a GSet<T> {
    type Item = &'a T;
    type IntoIter = std::collections::btree_set::Iter<'a, T>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.iter()
    }
}

impl<T: Ord + Clone + PartialEq> Lattice for GSet<T> {
    fn merge(&self, other: &Self) -> Self {
        Self(self.0.union(&other.0).cloned().collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lattice::assert_lattice_laws;

    #[test]
    fn new_is_empty() {
        let s: GSet<i32> = GSet::new();
        assert!(s.is_empty());
        assert_eq!(s.len(), 0);
    }

    #[test]
    fn insert_and_contains() {
        let mut s = GSet::new();
        assert!(s.insert(1));
        assert!(s.insert(2));
        assert!(!s.insert(1)); // duplicate
        assert!(s.contains(&1));
        assert!(s.contains(&2));
        assert!(!s.contains(&3));
        assert_eq!(s.len(), 2);
    }

    #[test]
    fn iter_sorted() {
        let mut s = GSet::new();
        s.insert(3);
        s.insert(1);
        s.insert(2);
        let items: Vec<_> = s.iter().copied().collect();
        assert_eq!(items, vec![1, 2, 3]);
    }

    #[test]
    fn into_iter() {
        let mut s = GSet::new();
        s.insert(3);
        s.insert(1);
        let items: Vec<_> = s.into_iter().collect();
        assert_eq!(items, vec![1, 3]);
    }

    #[test]
    fn lattice_laws() {
        let mut a = GSet::new();
        a.insert(1);
        a.insert(2);

        let mut b = GSet::new();
        b.insert(2);
        b.insert(3);

        let mut c = GSet::new();
        c.insert(3);
        c.insert(4);

        assert_lattice_laws(&a, &b, &c);
    }

    #[test]
    fn lattice_laws_disjoint() {
        let mut a = GSet::new();
        a.insert("x".to_string());

        let mut b = GSet::new();
        b.insert("y".to_string());

        let mut c = GSet::new();
        c.insert("z".to_string());

        assert_lattice_laws(&a, &b, &c);
    }

    #[test]
    fn merge_union() {
        let mut a = GSet::new();
        a.insert(1);
        a.insert(2);

        let mut b = GSet::new();
        b.insert(2);
        b.insert(3);

        let merged = a.merge(&b);
        assert_eq!(merged.len(), 3);
        assert!(merged.contains(&1));
        assert!(merged.contains(&2));
        assert!(merged.contains(&3));
    }

    #[test]
    fn merge_empty() {
        let a: GSet<i32> = GSet::new();
        let mut b = GSet::new();
        b.insert(1);

        assert_eq!(a.merge(&b).len(), 1);
        assert_eq!(b.merge(&a).len(), 1);
        assert_eq!(a.merge(&a).len(), 0);
    }

    #[test]
    fn serde_roundtrip() {
        let mut s = GSet::new();
        s.insert(3);
        s.insert(1);
        s.insert(2);

        let json = serde_json::to_value(&s).unwrap();
        assert_eq!(json, serde_json::json!([1, 2, 3]));

        let back: GSet<i32> = serde_json::from_value(json).unwrap();
        assert_eq!(back, s);
    }

    #[test]
    fn serde_empty() {
        let s: GSet<i32> = GSet::new();
        let json = serde_json::to_value(&s).unwrap();
        assert_eq!(json, serde_json::json!([]));

        let back: GSet<i32> = serde_json::from_value(json).unwrap();
        assert_eq!(back, s);
    }
}
