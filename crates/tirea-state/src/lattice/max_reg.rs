use serde::{Deserialize, Serialize};

use super::Lattice;

/// A register that merges by taking the maximum value.
///
/// Merge semantics: `max(a, b)`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct MaxReg<T: Ord>(T);

impl<T: Ord> MaxReg<T> {
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

impl<T: Ord + Clone + PartialEq> Lattice for MaxReg<T> {
    fn merge(&self, other: &Self) -> Self {
        if self.0 >= other.0 {
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
        let r = MaxReg::new(42);
        assert_eq!(*r.value(), 42);
    }

    #[test]
    fn set_replaces() {
        let mut r = MaxReg::new(1);
        r.set(100);
        assert_eq!(*r.value(), 100);
    }

    #[test]
    fn lattice_laws_i64() {
        let a = MaxReg::new(1i64);
        let b = MaxReg::new(5);
        let c = MaxReg::new(3);
        assert_lattice_laws(&a, &b, &c);

        let a = MaxReg::new(10i64);
        let b = MaxReg::new(10);
        let c = MaxReg::new(7);
        assert_lattice_laws(&a, &b, &c);
    }

    #[test]
    fn lattice_laws_string() {
        let a = MaxReg::new("alpha".to_string());
        let b = MaxReg::new("gamma".to_string());
        let c = MaxReg::new("beta".to_string());
        assert_lattice_laws(&a, &b, &c);
    }

    #[test]
    fn merge_takes_max() {
        let a = MaxReg::new(3);
        let b = MaxReg::new(7);
        assert_eq!(*a.merge(&b).value(), 7);
        assert_eq!(*b.merge(&a).value(), 7);
    }

    #[test]
    fn merge_equal_values() {
        let a = MaxReg::new(5);
        let b = MaxReg::new(5);
        assert_eq!(*a.merge(&b).value(), 5);
    }

    #[test]
    fn serde_roundtrip() {
        let r = MaxReg::new(42i64);
        let json = serde_json::to_value(&r).unwrap();
        assert_eq!(json, serde_json::json!(42));

        let back: MaxReg<i64> = serde_json::from_value(json).unwrap();
        assert_eq!(back, r);
    }

    #[test]
    fn serde_string_roundtrip() {
        let r = MaxReg::new("hello".to_string());
        let json = serde_json::to_value(&r).unwrap();
        assert_eq!(json, serde_json::json!("hello"));

        let back: MaxReg<String> = serde_json::from_value(json).unwrap();
        assert_eq!(back, r);
    }
}
