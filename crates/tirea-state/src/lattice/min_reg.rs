use serde::{Deserialize, Serialize};

use super::Lattice;

/// A register that merges by taking the minimum value.
///
/// Merge semantics: `min(a, b)`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct MinReg<T: Ord>(T);

impl<T: Ord> MinReg<T> {
    /// Create a new register with the given initial value.
    pub fn new(value: T) -> Self {
        Self(value)
    }

    /// Returns a reference to the current value.
    pub fn value(&self) -> &T {
        &self.0
    }

    /// Set a new value, unconditionally replacing the current one.
    pub fn set(&mut self, value: T) {
        self.0 = value;
    }
}

impl<T: Ord + Clone + PartialEq> Lattice for MinReg<T> {
    fn merge(&self, other: &Self) -> Self {
        if self.0 <= other.0 {
            self.clone()
        } else {
            other.clone()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lattice::assert_lattice_laws;

    #[test]
    fn new_and_value() {
        let r = MinReg::new(42);
        assert_eq!(*r.value(), 42);
    }

    #[test]
    fn set_replaces() {
        let mut r = MinReg::new(100);
        r.set(1);
        assert_eq!(*r.value(), 1);
    }

    #[test]
    fn lattice_laws_i64() {
        let a = MinReg::new(1i64);
        let b = MinReg::new(5);
        let c = MinReg::new(3);
        assert_lattice_laws(&a, &b, &c);

        let a = MinReg::new(10i64);
        let b = MinReg::new(10);
        let c = MinReg::new(7);
        assert_lattice_laws(&a, &b, &c);
    }

    #[test]
    fn lattice_laws_string() {
        let a = MinReg::new("alpha".to_string());
        let b = MinReg::new("gamma".to_string());
        let c = MinReg::new("beta".to_string());
        assert_lattice_laws(&a, &b, &c);
    }

    #[test]
    fn merge_takes_min() {
        let a = MinReg::new(3);
        let b = MinReg::new(7);
        assert_eq!(*a.merge(&b).value(), 3);
        assert_eq!(*b.merge(&a).value(), 3);
    }

    #[test]
    fn merge_equal_values() {
        let a = MinReg::new(5);
        let b = MinReg::new(5);
        assert_eq!(*a.merge(&b).value(), 5);
    }

    #[test]
    fn serde_roundtrip() {
        let r = MinReg::new(42i64);
        let json = serde_json::to_value(&r).unwrap();
        assert_eq!(json, serde_json::json!(42));

        let back: MinReg<i64> = serde_json::from_value(json).unwrap();
        assert_eq!(back, r);
    }

    #[test]
    fn serde_string_roundtrip() {
        let r = MinReg::new("hello".to_string());
        let json = serde_json::to_value(&r).unwrap();
        assert_eq!(json, serde_json::json!("hello"));

        let back: MinReg<String> = serde_json::from_value(json).unwrap();
        assert_eq!(back, r);
    }
}
