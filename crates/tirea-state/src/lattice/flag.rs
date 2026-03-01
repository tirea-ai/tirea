use serde::{Deserialize, Serialize};

use super::Lattice;

/// A monotonic boolean flag: once enabled, it stays enabled forever.
///
/// Merge semantics: logical OR.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Flag(bool);

impl Flag {
    /// Create a new disabled flag.
    pub fn new() -> Self {
        Self(false)
    }

    /// Create an already-enabled flag.
    pub fn enabled() -> Self {
        Self(true)
    }

    /// Enable the flag. This is irreversible.
    pub fn enable(&mut self) {
        self.0 = true;
    }

    /// Returns `true` if the flag has been enabled.
    pub fn is_enabled(&self) -> bool {
        self.0
    }
}

impl Default for Flag {
    fn default() -> Self {
        Self::new()
    }
}

impl Lattice for Flag {
    fn merge(&self, other: &Self) -> Self {
        Self(self.0 || other.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lattice::assert_lattice_laws;

    #[test]
    fn new_is_disabled() {
        let f = Flag::new();
        assert!(!f.is_enabled());
    }

    #[test]
    fn enable_sets_flag() {
        let mut f = Flag::new();
        f.enable();
        assert!(f.is_enabled());
    }

    #[test]
    fn enabled_constructor() {
        let f = Flag::enabled();
        assert!(f.is_enabled());
    }

    #[test]
    fn lattice_laws() {
        let a = Flag::new();
        let b = Flag::enabled();
        let c = Flag::new();
        assert_lattice_laws(&a, &b, &c);

        let a = Flag::enabled();
        let b = Flag::enabled();
        let c = Flag::new();
        assert_lattice_laws(&a, &b, &c);
    }

    #[test]
    fn merge_or_semantics() {
        let off = Flag::new();
        let on = Flag::enabled();

        assert_eq!(off.merge(&off), Flag::new());
        assert_eq!(off.merge(&on), Flag::enabled());
        assert_eq!(on.merge(&off), Flag::enabled());
        assert_eq!(on.merge(&on), Flag::enabled());
    }

    #[test]
    fn serde_roundtrip() {
        let f = Flag::enabled();
        let json = serde_json::to_value(&f).unwrap();
        assert_eq!(json, serde_json::Value::Bool(true));

        let back: Flag = serde_json::from_value(json).unwrap();
        assert_eq!(back, f);

        let f = Flag::new();
        let json = serde_json::to_value(&f).unwrap();
        assert_eq!(json, serde_json::Value::Bool(false));

        let back: Flag = serde_json::from_value(json).unwrap();
        assert_eq!(back, f);
    }
}
