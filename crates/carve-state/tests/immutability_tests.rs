//! Tests for immutability and deterministic replay.
//!
//! These tests verify that:
//! 1. apply_patch never mutates the original state
//! 2. Same (State, Patch) always produces the same result
//! 3. Accessors never mutate state, only generate patches

use carve_state::{apply_patch, AccessorOps, State};
use carve_state_derive::CarveViewModel;
use serde::{Deserialize, Serialize};
use serde_json::json;

#[derive(Debug, Clone, Serialize, Deserialize, CarveViewModel, PartialEq)]
struct GameState {
    score: i32,
    level: u32,
    player_name: String,
}

// ============================================================================
// Immutability Tests - apply_patch is pure
// ============================================================================

#[test]
fn test_apply_patch_does_not_mutate_original() {
    let original = json!({"score": 100, "level": 1, "player_name": "Alice"});
    let original_clone = original.clone();

    let accessor = GameState::access(&original);
    accessor.set_score(200);
    accessor.set_level(2);
    let patch = accessor.build();

    // Apply patch
    let _new_state = apply_patch(&original, &patch).unwrap();

    // Original state MUST be unchanged
    assert_eq!(original, original_clone, "apply_patch mutated the original state!");
    assert_eq!(original["score"], 100);
    assert_eq!(original["level"], 1);
}

#[test]
fn test_state_apply_creates_new_value() {
    let mut state = State::from_value(json!({"score": 50, "level": 1, "player_name": "Bob"}));
    let original_raw = state.raw().clone();

    let patch = {
        let accessor = state.access::<GameState>();
        accessor.set_score(150);
        accessor.build()
    };

    // Apply patch
    state.apply(&patch).unwrap();

    // The internal doc is replaced, but the old value is unchanged
    assert_ne!(state.raw(), &original_raw);
    assert_eq!(original_raw["score"], 50); // Original value unchanged
    assert_eq!(state.raw()["score"], 150); // New state has new value
}

#[test]
fn test_accessor_does_not_mutate_document() {
    let doc = json!({"score": 100, "level": 5, "player_name": "Charlie"});
    let doc_clone = doc.clone();

    // Create accessor and make changes
    let accessor = GameState::access(&doc);
    accessor.set_score(999);
    accessor.set_level(10);
    accessor.set_player_name("Modified".to_string());

    // Document MUST be unchanged (accessor only accumulates ops)
    assert_eq!(doc, doc_clone, "Accessor mutated the document!");
    assert_eq!(doc["score"], 100);
    assert_eq!(doc["level"], 5);
    assert_eq!(doc["player_name"], "Charlie");

    // Build patch and apply to verify changes are in the patch
    let patch = accessor.build();
    let new_doc = apply_patch(&doc, &patch).unwrap();
    assert_eq!(new_doc["score"], 999);
    assert_eq!(new_doc["level"], 10);
}

#[test]
fn test_multiple_accessors_same_document() {
    let doc = json!({"score": 100, "level": 1, "player_name": "Dave"});

    // Multiple accessors on the same document
    let accessor1 = GameState::access(&doc);
    accessor1.set_score(200);

    let accessor2 = GameState::access(&doc);
    accessor2.set_level(5);

    // Document still unchanged
    assert_eq!(doc["score"], 100);
    assert_eq!(doc["level"], 1);

    // Each accessor generates independent patches
    let patch1 = accessor1.build();
    let patch2 = accessor2.build();

    let result1 = apply_patch(&doc, &patch1).unwrap();
    assert_eq!(result1["score"], 200);
    assert_eq!(result1["level"], 1); // Unchanged

    let result2 = apply_patch(&doc, &patch2).unwrap();
    assert_eq!(result2["score"], 100); // Unchanged
    assert_eq!(result2["level"], 5);
}

// ============================================================================
// Deterministic Replay Tests
// ============================================================================

#[test]
fn test_deterministic_same_inputs_same_output() {
    let state = json!({"score": 10, "level": 1, "player_name": "Player1"});

    let accessor = GameState::access(&state);
    accessor.set_score(20);
    accessor.set_level(2);
    let patch = accessor.build();

    // Apply the same patch multiple times
    let result1 = apply_patch(&state, &patch).unwrap();
    let result2 = apply_patch(&state, &patch).unwrap();
    let result3 = apply_patch(&state, &patch).unwrap();

    // All results MUST be identical
    assert_eq!(result1, result2);
    assert_eq!(result2, result3);
    assert_eq!(result1["score"], 20);
    assert_eq!(result1["level"], 2);
}

#[test]
fn test_deterministic_replay_sequence() {
    let initial_state = json!({"score": 0, "level": 1, "player_name": "ReplayTest"});

    // Sequence of patches
    let patches = vec![
        {
            let accessor = GameState::access(&initial_state);
            accessor.set_score(10);
            accessor.build()
        },
        {
            let accessor = GameState::access(&initial_state);
            accessor.set_score(20);
            accessor.build()
        },
        {
            let accessor = GameState::access(&initial_state);
            accessor.set_level(2);
            accessor.build()
        },
    ];

    // Replay 1
    let mut state1 = initial_state.clone();
    for patch in &patches {
        state1 = apply_patch(&state1, patch).unwrap();
    }

    // Replay 2
    let mut state2 = initial_state.clone();
    for patch in &patches {
        state2 = apply_patch(&state2, patch).unwrap();
    }

    // Results MUST be identical
    assert_eq!(state1, state2);
    assert_eq!(state1["score"], 20);
    assert_eq!(state1["level"], 2);
}

#[test]
fn test_deterministic_complex_operations() {
    let initial = json!({
        "score": 100,
        "level": 1,
        "player_name": "Complex"
    });

    // Complex patch with multiple operations
    let patch = {
        let accessor = GameState::access(&initial);
        let mut score_field = accessor.score();
        score_field += 50; // Increment
        accessor.set_level(3);
        accessor.set_player_name("Updated".to_string());
        accessor.build()
    };

    // Apply 10 times from the same initial state
    let results: Vec<_> = (0..10)
        .map(|_| apply_patch(&initial, &patch).unwrap())
        .collect();

    // All results MUST be identical
    for result in &results {
        assert_eq!(result["score"], 150);
        assert_eq!(result["level"], 3);
        assert_eq!(result["player_name"], "Updated");
    }

    // First and last must be equal
    assert_eq!(results[0], results[9]);
}

// ============================================================================
// State History Replay Tests
// ============================================================================

#[test]
fn test_replay_state_history() {
    let mut state = State::from_value(json!({
        "score": 0,
        "level": 1,
        "player_name": "HistoryTest"
    }));

    let mut history = Vec::new();

    // Record patch history
    for i in 1u32..=5 {
        let patch = {
            let accessor = state.access::<GameState>();
            accessor.set_score((i * 10) as i32);
            accessor.set_level(i);
            accessor.build()
        };
        state.apply(&patch).unwrap();
        history.push(patch);
    }

    // Current state
    assert_eq!(state.raw()["score"], 50);
    assert_eq!(state.raw()["level"], 5);

    // Replay from initial state
    let initial = json!({"score": 0, "level": 1, "player_name": "HistoryTest"});
    let mut replayed = initial.clone();
    for patch in &history {
        replayed = apply_patch(&replayed, patch).unwrap();
    }

    // Replayed state MUST match current state
    assert_eq!(replayed, *state.raw());
    assert_eq!(replayed["score"], 50);
    assert_eq!(replayed["level"], 5);
}

#[test]
fn test_fork_and_replay() {
    let initial = json!({"score": 0, "level": 1, "player_name": "Fork"});

    // Timeline A
    let patch_a = {
        let accessor = GameState::access(&initial);
        accessor.set_score(100);
        accessor.set_player_name("Path A".to_string());
        accessor.build()
    };

    // Timeline B (different changes)
    let patch_b = {
        let accessor = GameState::access(&initial);
        accessor.set_score(200);
        accessor.set_level(5);
        accessor.build()
    };

    // Apply timeline A
    let state_a = apply_patch(&initial, &patch_a).unwrap();
    assert_eq!(state_a["score"], 100);
    assert_eq!(state_a["player_name"], "Path A");
    assert_eq!(state_a["level"], 1); // Unchanged

    // Apply timeline B
    let state_b = apply_patch(&initial, &patch_b).unwrap();
    assert_eq!(state_b["score"], 200);
    assert_eq!(state_b["level"], 5);
    assert_eq!(state_b["player_name"], "Fork"); // Unchanged

    // States are different
    assert_ne!(state_a, state_b);

    // But can replay each timeline deterministically
    let state_a_replay = apply_patch(&initial, &patch_a).unwrap();
    let state_b_replay = apply_patch(&initial, &patch_b).unwrap();

    assert_eq!(state_a, state_a_replay);
    assert_eq!(state_b, state_b_replay);
}

// ============================================================================
// Serialization and Replay Tests
// ============================================================================

#[test]
fn test_serialize_deserialize_replay() {
    let initial = json!({"score": 0, "level": 1, "player_name": "SerializeTest"});

    let patch = {
        let accessor = GameState::access(&initial);
        accessor.set_score(999);
        accessor.set_level(10);
        accessor.build()
    };

    // Serialize patch
    let patch_json = serde_json::to_string(&patch).unwrap();

    // Deserialize patch
    let deserialized_patch: carve_state::Patch = serde_json::from_str(&patch_json).unwrap();

    // Apply original and deserialized patches
    let result_original = apply_patch(&initial, &patch).unwrap();
    let result_deserialized = apply_patch(&initial, &deserialized_patch).unwrap();

    // Results MUST be identical
    assert_eq!(result_original, result_deserialized);
    assert_eq!(result_original["score"], 999);
    assert_eq!(result_original["level"], 10);
}

#[test]
fn test_patch_persistence_and_replay() {
    // Simulate storing patches for later replay
    let initial_state = State::from_value(json!({
        "score": 0,
        "level": 1,
        "player_name": "Persistent"
    }));

    // Generate and "persist" patches
    let patch1 = {
        let accessor = initial_state.access::<GameState>();
        accessor.set_score(10);
        accessor.build()
    };
    let patch1_str = serde_json::to_string(&patch1).unwrap();

    let patch2 = {
        let accessor = initial_state.access::<GameState>();
        accessor.set_level(2);
        accessor.build()
    };
    let patch2_str = serde_json::to_string(&patch2).unwrap();

    // Later: load patches and replay
    let loaded_patch1: carve_state::Patch = serde_json::from_str(&patch1_str).unwrap();
    let loaded_patch2: carve_state::Patch = serde_json::from_str(&patch2_str).unwrap();

    let mut replayed_state = initial_state.raw().clone();
    replayed_state = apply_patch(&replayed_state, &loaded_patch1).unwrap();
    replayed_state = apply_patch(&replayed_state, &loaded_patch2).unwrap();

    assert_eq!(replayed_state["score"], 10);
    assert_eq!(replayed_state["level"], 2);
    assert_eq!(replayed_state["player_name"], "Persistent");
}

// ============================================================================
// Property: Commutativity and Order Independence (where applicable)
// ============================================================================

#[test]
fn test_independent_patches_commute() {
    let initial = json!({"score": 0, "level": 1, "player_name": "Commute"});

    // Two independent patches (different fields)
    let patch_score = {
        let accessor = GameState::access(&initial);
        accessor.set_score(100);
        accessor.build()
    };

    let patch_level = {
        let accessor = GameState::access(&initial);
        accessor.set_level(5);
        accessor.build()
    };

    // Apply in order: score then level
    let mut state1 = initial.clone();
    state1 = apply_patch(&state1, &patch_score).unwrap();
    state1 = apply_patch(&state1, &patch_level).unwrap();

    // Apply in order: level then score
    let mut state2 = initial.clone();
    state2 = apply_patch(&state2, &patch_level).unwrap();
    state2 = apply_patch(&state2, &patch_score).unwrap();

    // Results MUST be the same (independent operations commute)
    assert_eq!(state1, state2);
    assert_eq!(state1["score"], 100);
    assert_eq!(state1["level"], 5);
}

#[test]
fn test_dependent_patches_order_matters() {
    let initial = json!({"score": 10, "level": 1, "player_name": "Order"});

    // Two dependent patches (same field)
    let patch1 = {
        let accessor = GameState::access(&initial);
        accessor.set_score(20);
        accessor.build()
    };

    let patch2 = {
        let accessor = GameState::access(&initial);
        accessor.set_score(30);
        accessor.build()
    };

    // Order matters: patch2 overwrites patch1
    let mut state1 = initial.clone();
    state1 = apply_patch(&state1, &patch1).unwrap();
    state1 = apply_patch(&state1, &patch2).unwrap();

    let mut state2 = initial.clone();
    state2 = apply_patch(&state2, &patch2).unwrap();
    state2 = apply_patch(&state2, &patch1).unwrap();

    // Different results (last write wins)
    assert_ne!(state1, state2);
    assert_eq!(state1["score"], 30); // patch2 applied last
    assert_eq!(state2["score"], 20); // patch1 applied last
}
