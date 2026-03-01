use std::collections::BTreeMap;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use super::Lattice;

/// An observed-remove set (OR-Set) with add-wins semantics.
///
/// Each element is tracked with a timestamp. Removals record a tombstone timestamp.
/// An element is considered present when its entry timestamp is greater than or equal
/// to its tombstone timestamp (add-wins: concurrent add and remove resolve in favor
/// of add).
///
/// The internal clock advances automatically on mutation and is bumped to
/// `max(self, other)` on merge.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ORSet<T: Ord> {
    entries: BTreeMap<T, u64>,
    tombstones: BTreeMap<T, u64>,
    #[serde(skip)]
    clock: u64,
}

impl<'de, T: Ord + DeserializeOwned> Deserialize<'de> for ORSet<T> {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct Raw<T: Ord> {
            entries: BTreeMap<T, u64>,
            tombstones: BTreeMap<T, u64>,
        }

        let raw = Raw::deserialize(deserializer)?;
        let max_entry = raw.entries.values().copied().max().unwrap_or(0);
        let max_tomb = raw.tombstones.values().copied().max().unwrap_or(0);
        let clock = max_entry.max(max_tomb);

        Ok(Self {
            entries: raw.entries,
            tombstones: raw.tombstones,
            clock,
        })
    }
}

impl<T: Ord> ORSet<T> {
    /// Create an empty OR-Set.
    pub fn new() -> Self {
        Self {
            entries: BTreeMap::new(),
            tombstones: BTreeMap::new(),
            clock: 0,
        }
    }

    /// Insert an element. If previously removed, the add-wins semantics re-add it.
    pub fn insert(&mut self, value: T) {
        self.clock += 1;
        self.entries.insert(value, self.clock);
    }

    /// Remove an element by recording a tombstone at the current clock.
    pub fn remove(&mut self, value: &T)
    where
        T: Clone,
    {
        if self.entries.contains_key(value) {
            self.clock += 1;
            self.tombstones.insert(value.clone(), self.clock);
        }
    }

    /// Returns `true` if the element is present (entry timestamp >= tombstone timestamp).
    pub fn contains(&self, value: &T) -> bool {
        match self.entries.get(value) {
            Some(&entry_ts) => {
                let tomb_ts = self.tombstones.get(value).copied().unwrap_or(0);
                entry_ts >= tomb_ts
            }
            None => false,
        }
    }

    /// Returns a sorted vector of references to all present elements.
    pub fn elements(&self) -> Vec<&T> {
        self.entries
            .iter()
            .filter(|(k, &entry_ts)| {
                let tomb_ts = self.tombstones.get(*k).copied().unwrap_or(0);
                entry_ts >= tomb_ts
            })
            .map(|(k, _)| k)
            .collect()
    }

    /// Returns the number of present elements.
    pub fn len(&self) -> usize {
        self.elements().len()
    }

    /// Returns `true` if no elements are present.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn max_observed_ts(&self) -> u64 {
        let max_entry = self.entries.values().copied().max().unwrap_or(0);
        let max_tomb = self.tombstones.values().copied().max().unwrap_or(0);
        max_entry.max(max_tomb)
    }
}

impl<T: Ord> Default for ORSet<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T: Ord + Clone + PartialEq> Lattice for ORSet<T> {
    fn merge(&self, other: &Self) -> Self {
        let mut entries = BTreeMap::new();
        let mut tombstones = BTreeMap::new();

        // Merge entries: per-element max timestamp
        for (k, &ts) in &self.entries {
            entries.insert(k.clone(), ts);
        }
        for (k, &ts) in &other.entries {
            let entry = entries.entry(k.clone()).or_insert(0);
            *entry = (*entry).max(ts);
        }

        // Merge tombstones: per-element max timestamp
        for (k, &ts) in &self.tombstones {
            tombstones.insert(k.clone(), ts);
        }
        for (k, &ts) in &other.tombstones {
            let entry = tombstones.entry(k.clone()).or_insert(0);
            *entry = (*entry).max(ts);
        }

        let clock = self.max_observed_ts().max(other.max_observed_ts());

        Self {
            entries,
            tombstones,
            clock,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lattice::assert_lattice_laws;

    #[test]
    fn new_is_empty() {
        let s: ORSet<i32> = ORSet::new();
        assert!(s.is_empty());
        assert_eq!(s.len(), 0);
    }

    #[test]
    fn insert_and_contains() {
        let mut s = ORSet::new();
        s.insert(1);
        s.insert(2);
        assert!(s.contains(&1));
        assert!(s.contains(&2));
        assert!(!s.contains(&3));
        assert_eq!(s.len(), 2);
    }

    #[test]
    fn remove_element() {
        let mut s = ORSet::new();
        s.insert(1);
        s.insert(2);
        s.remove(&1);
        assert!(!s.contains(&1));
        assert!(s.contains(&2));
        assert_eq!(s.len(), 1);
    }

    #[test]
    fn remove_nonexistent_is_noop() {
        let mut s = ORSet::new();
        s.insert(1);
        s.remove(&2); // no-op
        assert_eq!(s.len(), 1);
    }

    #[test]
    fn add_wins_after_concurrent_remove() {
        // Simulate: replica A removes x, replica B adds x concurrently
        let mut a = ORSet::new();
        a.insert("x".to_string());

        let mut b = a.clone();

        // A removes
        a.remove(&"x".to_string());
        assert!(!a.contains(&"x".to_string()));

        // B re-adds (concurrent with A's remove)
        b.insert("x".to_string());
        assert!(b.contains(&"x".to_string()));

        // Merge: add-wins
        let merged = a.merge(&b);
        assert!(
            merged.contains(&"x".to_string()),
            "add-wins: element should be present after merge"
        );
    }

    #[test]
    fn elements_sorted() {
        let mut s = ORSet::new();
        s.insert(3);
        s.insert(1);
        s.insert(2);
        assert_eq!(s.elements(), vec![&1, &2, &3]);
    }

    #[test]
    fn lattice_laws() {
        let mut a = ORSet::new();
        a.insert(1);
        a.insert(2);

        let mut b = ORSet::new();
        b.insert(2);
        b.insert(3);

        let mut c = ORSet::new();
        c.insert(1);
        c.insert(3);

        assert_lattice_laws(&a, &b, &c);
    }

    #[test]
    fn lattice_laws_with_removes() {
        let mut a = ORSet::new();
        a.insert(1);
        a.insert(2);
        a.remove(&1);

        let mut b = ORSet::new();
        b.insert(2);
        b.insert(3);
        b.remove(&3);

        let mut c = ORSet::new();
        c.insert(1);
        c.insert(3);

        assert_lattice_laws(&a, &b, &c);
    }

    #[test]
    fn merge_union_of_entries() {
        let mut a = ORSet::new();
        a.insert(1);
        a.insert(2);

        let mut b = ORSet::new();
        b.insert(3);
        b.insert(4);

        let merged = a.merge(&b);
        assert_eq!(merged.len(), 4);
    }

    #[test]
    fn merge_empty_sets() {
        let a: ORSet<i32> = ORSet::new();
        let b: ORSet<i32> = ORSet::new();
        let merged = a.merge(&b);
        assert!(merged.is_empty());
    }

    #[test]
    fn merge_one_empty() {
        let a: ORSet<i32> = ORSet::new();
        let mut b = ORSet::new();
        b.insert(1);

        assert_eq!(a.merge(&b).len(), 1);
        assert_eq!(b.merge(&a).len(), 1);
    }

    #[test]
    fn serde_roundtrip() {
        let mut s = ORSet::new();
        s.insert(1);
        s.insert(2);
        s.remove(&1);

        let json = serde_json::to_string(&s).unwrap();
        let back: ORSet<i32> = serde_json::from_str(&json).unwrap();

        // After deserialization, the "visible" state should match
        assert_eq!(s.elements(), back.elements());
        assert_eq!(s.entries, back.entries);
        assert_eq!(s.tombstones, back.tombstones);
    }

    #[test]
    fn serde_preserves_tombstones() {
        let mut s = ORSet::new();
        s.insert("a".to_string());
        s.remove(&"a".to_string());

        let json = serde_json::to_value(&s).unwrap();
        // Should have both entries and tombstones in the JSON
        assert!(json.get("entries").is_some());
        assert!(json.get("tombstones").is_some());

        let back: ORSet<String> = serde_json::from_value(json).unwrap();
        assert!(!back.contains(&"a".to_string()));
    }
}
