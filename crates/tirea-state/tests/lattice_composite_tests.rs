//! Integration tests for `#[derive(Lattice)]`, ORSet/ORMap clock reconstruction.
#![allow(missing_docs)]

use tirea_state::{Flag, GCounter, GSet, Lattice, MaxReg, ORMap, ORSet};

// ============================================================================
// derive(Lattice) — simple struct
// ============================================================================

#[derive(Debug, Clone, PartialEq, Lattice)]
struct Dashboard {
    visitors: GCounter,
    published: Flag,
    tags: GSet<String>,
}

#[test]
fn test_derive_lattice_simple() {
    let mut a = Dashboard {
        visitors: GCounter::new(),
        published: Flag::new(),
        tags: GSet::new(),
    };
    a.visitors.increment("node-a", 10);
    a.published.enable();
    a.tags.insert("rust".to_string());

    let mut b = Dashboard {
        visitors: GCounter::new(),
        published: Flag::new(),
        tags: GSet::new(),
    };
    b.visitors.increment("node-b", 5);
    b.tags.insert("crdt".to_string());

    let merged = a.merge(&b);
    assert_eq!(merged.visitors.value(), 15);
    assert!(merged.published.is_enabled());
    assert!(merged.tags.contains(&"rust".to_string()));
    assert!(merged.tags.contains(&"crdt".to_string()));
    assert_eq!(merged.tags.len(), 2);
}

// ============================================================================
// derive(Lattice) — generic struct
// ============================================================================

#[derive(Debug, Clone, PartialEq, Lattice)]
struct Wrapper<T: Lattice> {
    inner: T,
    flag: Flag,
}

#[test]
fn test_derive_lattice_generic() {
    let a = Wrapper {
        inner: MaxReg::new(10i64),
        flag: Flag::new(),
    };
    let mut b = Wrapper {
        inner: MaxReg::new(20i64),
        flag: Flag::new(),
    };
    b.flag.enable();

    let merged = a.merge(&b);
    assert_eq!(*merged.inner.value(), 20);
    assert!(merged.flag.is_enabled());
}

// ============================================================================
// derive(Lattice) — nested composed struct
// ============================================================================

#[derive(Debug, Clone, PartialEq, Lattice)]
struct Inner {
    score: MaxReg<i64>,
}

#[derive(Debug, Clone, PartialEq, Lattice)]
struct Outer {
    data: Inner,
    enabled: Flag,
}

#[test]
fn test_derive_lattice_nested() {
    let a = Outer {
        data: Inner {
            score: MaxReg::new(10),
        },
        enabled: Flag::new(),
    };
    let mut b = Outer {
        data: Inner {
            score: MaxReg::new(20),
        },
        enabled: Flag::new(),
    };
    b.enabled.enable();

    let merged = a.merge(&b);
    assert_eq!(*merged.data.score.value(), 20);
    assert!(merged.enabled.is_enabled());
}

#[test]
fn test_derive_lattice_nested_as_ormap_value() {
    let mut map: ORMap<String, Inner> = ORMap::new();
    map.put(
        "player1".to_string(),
        Inner {
            score: MaxReg::new(100),
        },
    );

    let mut map2: ORMap<String, Inner> = ORMap::new();
    map2.put(
        "player1".to_string(),
        Inner {
            score: MaxReg::new(200),
        },
    );

    let merged = map.merge(&map2);
    let val = merged.get(&"player1".to_string()).unwrap();
    assert_eq!(*val.score.value(), 200);
}

// ============================================================================
// ORSet clock reconstruction after serde round-trip
// ============================================================================

#[test]
fn test_orset_clock_after_serde() {
    let mut set = ORSet::new();
    set.insert(1);
    set.insert(2);
    set.remove(&1);
    // clock is now 3

    let json = serde_json::to_string(&set).unwrap();
    let mut deserialized: ORSet<i32> = serde_json::from_str(&json).unwrap();

    // Insert after deserialization must use a timestamp > 3
    deserialized.insert(3);

    // Verify the new element is present
    assert!(deserialized.contains(&3));
    // Verify original state is preserved
    assert!(!deserialized.contains(&1)); // was removed
    assert!(deserialized.contains(&2));

    // The new insert should not collide with stored timestamps.
    // Merge the original with the deserialized-then-mutated version:
    // if clock was properly reconstructed, element 3 survives.
    let merged = set.merge(&deserialized);
    assert!(merged.contains(&3));
    assert!(merged.contains(&2));
    assert!(!merged.contains(&1));
}

// ============================================================================
// ORMap clock reconstruction after serde round-trip
// ============================================================================

#[test]
fn test_ormap_clock_after_serde() {
    let mut map: ORMap<String, MaxReg<i64>> = ORMap::new();
    map.put("a".to_string(), MaxReg::new(10));
    map.put("b".to_string(), MaxReg::new(20));
    map.remove(&"a".to_string());
    // clock is now 3

    let json = serde_json::to_string(&map).unwrap();
    let mut deserialized: ORMap<String, MaxReg<i64>> = serde_json::from_str(&json).unwrap();

    // Put after deserialization must use a timestamp > 3
    deserialized.put("c".to_string(), MaxReg::new(30));

    // Verify the new entry is present
    assert!(deserialized.contains_key(&"c".to_string()));
    // Verify original state is preserved
    assert!(!deserialized.contains_key(&"a".to_string())); // was removed
    assert!(deserialized.contains_key(&"b".to_string()));

    // Merge the original with the deserialized-then-mutated version:
    // if clock was properly reconstructed, key "c" survives.
    let merged = map.merge(&deserialized);
    assert!(merged.contains_key(&"c".to_string()));
    assert!(merged.contains_key(&"b".to_string()));
    assert!(!merged.contains_key(&"a".to_string()));
}
