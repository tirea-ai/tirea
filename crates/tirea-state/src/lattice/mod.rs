//! Lattice trait and CRDT primitive types for conflict-free parallel state merging.
//!
//! A lattice is a partially ordered set where every pair of elements has a least upper bound
//! (join/merge). The [`Lattice`] trait captures this via the [`merge`](Lattice::merge) method,
//! which must satisfy commutativity, associativity, and idempotency.
//!
//! # Primitive Types
//!
//! | Type | Merge semantics |
//! |------|----------------|
//! | [`Flag`] | Boolean OR (monotonic enable) |
//! | [`MaxReg<T>`] | Maximum value |
//! | [`MinReg<T>`] | Minimum value |
//! | [`LWWReg<T>`] | Last-writer-wins by `(timestamp, value)` |
//! | [`GCounter`] | Grow-only counter (per-node max, sum for value) |
//! | [`GSet<T>`] | Grow-only set (union) |
//! | [`ORSet<T>`] | Observed-remove set (add-wins) |
//! | [`ORMap<K, V>`] | Observed-remove map with recursive value merge (put-wins) |

mod flag;
mod g_counter;
mod g_set;
mod lww_reg;
mod max_reg;
mod min_reg;
mod or_map;
mod or_set;

pub use flag::Flag;
pub use g_counter::GCounter;
pub use g_set::GSet;
pub use lww_reg::LWWReg;
pub use max_reg::MaxReg;
pub use min_reg::MinReg;
pub use or_map::ORMap;
pub use or_set::ORSet;

/// A join-semilattice: a set equipped with a commutative, associative, idempotent merge.
///
/// Implementors must guarantee the following laws for all values `a`, `b`, `c`:
///
/// - **Commutativity**: `a.merge(&b) == b.merge(&a)`
/// - **Associativity**: `a.merge(&b).merge(&c) == a.merge(&b.merge(&c))`
/// - **Idempotency**: `a.merge(&a) == a`
pub trait Lattice: Clone + PartialEq {
    /// Compute the least upper bound of `self` and `other`.
    fn merge(&self, other: &Self) -> Self;
}

/// Assert the three lattice laws (commutativity, associativity, idempotency) for a triple of values.
///
/// Intended for use in `#[cfg(test)]` modules.
///
/// # Panics
///
/// Panics with a descriptive message if any law is violated.
#[cfg(test)]
pub(crate) fn assert_lattice_laws<T: Lattice + std::fmt::Debug>(a: &T, b: &T, c: &T) {
    // Commutativity: a ∨ b == b ∨ a
    assert_eq!(
        a.merge(b),
        b.merge(a),
        "commutativity violated: a.merge(b) != b.merge(a)\n  a = {a:?}\n  b = {b:?}"
    );

    // Associativity: (a ∨ b) ∨ c == a ∨ (b ∨ c)
    assert_eq!(
        a.merge(b).merge(c),
        a.merge(&b.merge(c)),
        "associativity violated: (a∨b)∨c != a∨(b∨c)\n  a = {a:?}\n  b = {b:?}\n  c = {c:?}"
    );

    // Idempotency: a ∨ a == a
    assert_eq!(
        a.merge(a),
        a.clone(),
        "idempotency violated for a: a.merge(a) != a\n  a = {a:?}"
    );
    assert_eq!(
        b.merge(b),
        b.clone(),
        "idempotency violated for b: b.merge(b) != b\n  b = {b:?}"
    );
    assert_eq!(
        c.merge(c),
        c.clone(),
        "idempotency violated for c: c.merge(c) != c\n  c = {c:?}"
    );
}
