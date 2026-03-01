use serde::{Deserialize, Serialize};

use super::Lattice;

/// A last-writer-wins register with an internal monotonic clock.
///
/// Merge semantics: the entry with the larger `(timestamp, value)` tuple wins,
/// providing a deterministic tiebreaker when timestamps are equal.
///
/// The internal clock advances automatically on each [`set`](LWWReg::set) call
/// and is bumped to `max(self, other)` on merge, ensuring subsequent mutations
/// produce timestamps greater than all previously observed values.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LWWReg<T: Ord> {
    #[serde(rename = "v")]
    value: T,
    ts: u64,
}

impl<T: Ord + PartialEq> PartialEq for LWWReg<T> {
    fn eq(&self, other: &Self) -> bool {
        self.value == other.value && self.ts == other.ts
    }
}

impl<T: Ord + Eq> Eq for LWWReg<T> {}

impl<T: Ord> LWWReg<T> {
    /// Create a new register with the given value and timestamp 0.
    pub fn new(value: T) -> Self {
        Self { value, ts: 0 }
    }

    /// Returns a reference to the current value.
    pub fn value(&self) -> &T {
        &self.value
    }

    /// Set a new value, advancing the internal clock.
    pub fn set(&mut self, value: T) {
        self.ts += 1;
        self.value = value;
    }

    /// Returns the current timestamp.
    pub fn timestamp(&self) -> u64 {
        self.ts
    }
}

impl<T: Ord + Clone + PartialEq> Lattice for LWWReg<T> {
    fn merge(&self, other: &Self) -> Self {
        let winner = match self.ts.cmp(&other.ts) {
            std::cmp::Ordering::Greater => self,
            std::cmp::Ordering::Less => other,
            std::cmp::Ordering::Equal => {
                // Deterministic tiebreaker by value
                if self.value >= other.value {
                    self
                } else {
                    other
                }
            }
        };
        let max_ts = self.ts.max(other.ts);
        Self {
            value: winner.value.clone(),
            ts: max_ts,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lattice::assert_lattice_laws;

    #[test]
    fn new_and_value() {
        let r = LWWReg::new(42);
        assert_eq!(*r.value(), 42);
        assert_eq!(r.timestamp(), 0);
    }

    #[test]
    fn set_advances_clock() {
        let mut r = LWWReg::new(1);
        r.set(2);
        assert_eq!(*r.value(), 2);
        assert_eq!(r.timestamp(), 1);
        r.set(3);
        assert_eq!(*r.value(), 3);
        assert_eq!(r.timestamp(), 2);
    }

    #[test]
    fn lattice_laws_different_timestamps() {
        let mut a = LWWReg::new(10i64);
        let mut b = LWWReg::new(20i64);
        b.set(25);
        let mut c = LWWReg::new(30i64);
        c.set(35);
        c.set(40);
        assert_lattice_laws(&a, &b, &c);

        // Another triple
        a.set(100);
        a.set(200);
        a.set(300);
        assert_lattice_laws(&a, &b, &c);
    }

    #[test]
    fn lattice_laws_same_timestamp() {
        // All at ts=0, different values — tiebreaker by value
        let a = LWWReg::new(1i64);
        let b = LWWReg::new(2i64);
        let c = LWWReg::new(3i64);
        assert_lattice_laws(&a, &b, &c);
    }

    #[test]
    fn merge_higher_timestamp_wins() {
        let a = LWWReg::new(100i64);
        let mut b = LWWReg::new(1i64);
        b.set(2);

        let merged = a.merge(&b);
        assert_eq!(*merged.value(), 2);
        assert_eq!(merged.timestamp(), 1);
    }

    #[test]
    fn merge_same_timestamp_tiebreak_by_value() {
        let a = LWWReg::new(10i64);
        let b = LWWReg::new(20i64);
        // Both at ts=0, value 20 > 10 => 20 wins
        let merged = a.merge(&b);
        assert_eq!(*merged.value(), 20);

        // Commutative
        let merged2 = b.merge(&a);
        assert_eq!(*merged2.value(), 20);
    }

    #[test]
    fn merge_bumps_clock_to_max() {
        let mut a = LWWReg::new(1i64);
        a.set(2); // ts=1
        a.set(3); // ts=2

        let mut b = LWWReg::new(10i64);
        b.set(20); // ts=1

        let merged = a.merge(&b);
        // a wins (ts=2 > ts=1), clock = max(2,1) = 2
        assert_eq!(*merged.value(), 3);
        assert_eq!(merged.timestamp(), 2);
    }

    #[test]
    fn serde_roundtrip() {
        let mut r = LWWReg::new(42i64);
        r.set(99);
        let json = serde_json::to_value(&r).unwrap();
        assert_eq!(json, serde_json::json!({"v": 99, "ts": 1}));

        let back: LWWReg<i64> = serde_json::from_value(json).unwrap();
        assert_eq!(back, r);
    }

    #[test]
    fn serde_string_roundtrip() {
        let r = LWWReg::new("hello".to_string());
        let json = serde_json::to_value(&r).unwrap();
        assert_eq!(json, serde_json::json!({"v": "hello", "ts": 0}));

        let back: LWWReg<String> = serde_json::from_value(json).unwrap();
        assert_eq!(back, r);
    }
}
