use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::Lattice;

/// A grow-only counter (G-Counter) with per-node tracking.
///
/// Each node independently increments its own counter. The total value is the
/// sum of all per-node counters. Merge takes the per-node maximum.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct GCounter(BTreeMap<String, u64>);

impl GCounter {
    /// Create an empty counter with value 0.
    pub fn new() -> Self {
        Self(BTreeMap::new())
    }

    /// Increment the counter for the given node by `delta`.
    pub fn increment(&mut self, node: &str, delta: u64) {
        let entry = self.0.entry(node.to_string()).or_insert(0);
        *entry = entry.saturating_add(delta);
    }

    /// Returns the total counter value (sum of all nodes).
    pub fn value(&self) -> u64 {
        self.0.values().sum()
    }

    /// Returns the counter value for a specific node.
    pub fn node_value(&self, node: &str) -> u64 {
        self.0.get(node).copied().unwrap_or(0)
    }
}

impl Default for GCounter {
    fn default() -> Self {
        Self::new()
    }
}

impl Lattice for GCounter {
    fn merge(&self, other: &Self) -> Self {
        let mut result = self.0.clone();
        for (node, &count) in &other.0 {
            let entry = result.entry(node.clone()).or_insert(0);
            *entry = (*entry).max(count);
        }
        Self(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lattice::assert_lattice_laws;

    #[test]
    fn new_is_zero() {
        let c = GCounter::new();
        assert_eq!(c.value(), 0);
    }

    #[test]
    fn increment_and_value() {
        let mut c = GCounter::new();
        c.increment("a", 3);
        c.increment("b", 5);
        assert_eq!(c.value(), 8);
        assert_eq!(c.node_value("a"), 3);
        assert_eq!(c.node_value("b"), 5);
        assert_eq!(c.node_value("c"), 0);
    }

    #[test]
    fn increment_same_node() {
        let mut c = GCounter::new();
        c.increment("a", 3);
        c.increment("a", 7);
        assert_eq!(c.node_value("a"), 10);
        assert_eq!(c.value(), 10);
    }

    #[test]
    fn lattice_laws() {
        let mut a = GCounter::new();
        a.increment("x", 3);

        let mut b = GCounter::new();
        b.increment("y", 5);

        let mut c = GCounter::new();
        c.increment("x", 1);
        c.increment("y", 2);

        assert_lattice_laws(&a, &b, &c);
    }

    #[test]
    fn lattice_laws_overlapping_nodes() {
        let mut a = GCounter::new();
        a.increment("n", 10);

        let mut b = GCounter::new();
        b.increment("n", 7);
        b.increment("m", 3);

        let mut c = GCounter::new();
        c.increment("n", 5);
        c.increment("m", 8);

        assert_lattice_laws(&a, &b, &c);
    }

    #[test]
    fn merge_per_node_max() {
        let mut a = GCounter::new();
        a.increment("x", 3);
        a.increment("y", 1);

        let mut b = GCounter::new();
        b.increment("x", 1);
        b.increment("y", 5);
        b.increment("z", 2);

        let merged = a.merge(&b);
        assert_eq!(merged.node_value("x"), 3);
        assert_eq!(merged.node_value("y"), 5);
        assert_eq!(merged.node_value("z"), 2);
        assert_eq!(merged.value(), 10);
    }

    #[test]
    fn merge_empty() {
        let a = GCounter::new();
        let mut b = GCounter::new();
        b.increment("x", 5);

        assert_eq!(a.merge(&b).value(), 5);
        assert_eq!(b.merge(&a).value(), 5);
        assert_eq!(a.merge(&a).value(), 0);
    }

    #[test]
    fn serde_roundtrip() {
        let mut c = GCounter::new();
        c.increment("a", 3);
        c.increment("b", 5);

        let json = serde_json::to_value(&c).unwrap();
        assert_eq!(json, serde_json::json!({"a": 3, "b": 5}));

        let back: GCounter = serde_json::from_value(json).unwrap();
        assert_eq!(back, c);
    }

    #[test]
    fn serde_empty() {
        let c = GCounter::new();
        let json = serde_json::to_value(&c).unwrap();
        assert_eq!(json, serde_json::json!({}));

        let back: GCounter = serde_json::from_value(json).unwrap();
        assert_eq!(back, c);
    }
}
