//! Integration tests for StateManager.
//!
//! These tests verify the StateManager's ability to manage immutable state
//! with patch history and replay capabilities.

use carve_state::{path, Context, Op, Patch, StateManager, TrackedPatch};
use carve_state_derive::State;
use serde::{Deserialize, Serialize};
use serde_json::json;

// ============================================================================
// Test state types
// ============================================================================

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, State)]
struct CounterState {
    value: i64,
    label: String,
}

// ============================================================================
// Helper functions
// ============================================================================

fn make_patch(ops: Vec<Op>, source: &str) -> TrackedPatch {
    TrackedPatch::new(Patch::with_ops(ops)).with_source(source)
}

// ============================================================================
// StateManager basic tests
// ============================================================================

#[tokio::test]
async fn test_manager_new_and_snapshot() {
    let initial = json!({"version": 1, "status": "init"});
    let manager = StateManager::new(initial.clone());

    let snapshot = manager.snapshot().await;
    assert_eq!(snapshot, initial);
}

#[tokio::test]
async fn test_manager_commit_single() {
    let manager = StateManager::new(json!({"version": 1, "status": "init"}));

    let patch = make_patch(
        vec![
            Op::set(path!("version"), json!(2)),
            Op::set(path!("status"), json!("updated")),
        ],
        "test:update",
    );

    let result = manager.commit(patch).await.unwrap();
    assert_eq!(result.patches_applied, 1);
    assert_eq!(result.ops_applied, 2);

    let snapshot = manager.snapshot().await;
    assert_eq!(snapshot["version"], 2);
    assert_eq!(snapshot["status"], "updated");
}

#[tokio::test]
async fn test_manager_commit_multiple() {
    let manager = StateManager::new(json!({"value": 0}));

    // Apply multiple patches sequentially
    for i in 1..=5 {
        let patch = make_patch(
            vec![Op::set(path!("value"), json!(i))],
            &format!("step_{}", i),
        );
        manager.commit(patch).await.unwrap();
    }

    let snapshot = manager.snapshot().await;
    assert_eq!(snapshot["value"], 5);

    let history = manager.history().await;
    assert_eq!(history.len(), 5);
}

#[tokio::test]
async fn test_manager_commit_batch() {
    let manager = StateManager::new(json!({}));

    let patches = vec![
        make_patch(vec![Op::set(path!("a"), json!(1))], "s1"),
        make_patch(vec![Op::set(path!("b"), json!(2))], "s2"),
        make_patch(vec![Op::set(path!("c"), json!(3))], "s3"),
    ];

    let result = manager.commit_batch(patches).await.unwrap();
    assert_eq!(result.patches_applied, 3);
    assert_eq!(result.ops_applied, 3);

    let snapshot = manager.snapshot().await;
    assert_eq!(snapshot["a"], 1);
    assert_eq!(snapshot["b"], 2);
    assert_eq!(snapshot["c"], 3);
}

#[tokio::test]
async fn test_manager_commit_batch_empty() {
    let manager = StateManager::new(json!({"x": 1}));

    let result = manager.commit_batch(vec![]).await.unwrap();
    assert_eq!(result.patches_applied, 0);
    assert_eq!(result.ops_applied, 0);

    let snapshot = manager.snapshot().await;
    assert_eq!(snapshot["x"], 1);
}

// ============================================================================
// History and replay tests
// ============================================================================

#[tokio::test]
async fn test_manager_history() {
    let manager = StateManager::new(json!({}));

    manager
        .commit(make_patch(vec![Op::set(path!("x"), json!(1))], "step1"))
        .await
        .unwrap();

    manager
        .commit(make_patch(vec![Op::set(path!("y"), json!(2))], "step2"))
        .await
        .unwrap();

    manager
        .commit(make_patch(vec![Op::set(path!("z"), json!(3))], "step3"))
        .await
        .unwrap();

    let history = manager.history().await;
    assert_eq!(history.len(), 3);
    assert_eq!(history[0].source.as_deref(), Some("step1"));
    assert_eq!(history[1].source.as_deref(), Some("step2"));
    assert_eq!(history[2].source.as_deref(), Some("step3"));
}

#[tokio::test]
async fn test_manager_history_len() {
    let manager = StateManager::new(json!({}));

    assert_eq!(manager.history_len().await, 0);

    manager
        .commit(make_patch(vec![Op::set(path!("x"), json!(1))], "s1"))
        .await
        .unwrap();

    assert_eq!(manager.history_len().await, 1);

    manager
        .commit(make_patch(vec![Op::set(path!("y"), json!(2))], "s2"))
        .await
        .unwrap();

    assert_eq!(manager.history_len().await, 2);
}

#[tokio::test]
async fn test_manager_replay_to() {
    let manager = StateManager::new(json!({}));

    // Apply several patches that modify the same field
    manager
        .commit(make_patch(vec![Op::set(path!("count"), json!(10))], "s1"))
        .await
        .unwrap();

    manager
        .commit(make_patch(vec![Op::set(path!("count"), json!(20))], "s2"))
        .await
        .unwrap();

    manager
        .commit(make_patch(vec![Op::set(path!("count"), json!(30))], "s3"))
        .await
        .unwrap();

    // Replay to index 0 (after first patch)
    let state0 = manager.replay_to(0).await.unwrap();
    assert_eq!(state0["count"], 10);

    // Replay to index 1 (after second patch)
    let state1 = manager.replay_to(1).await.unwrap();
    assert_eq!(state1["count"], 20);

    // Replay to index 2 (after third patch)
    let state2 = manager.replay_to(2).await.unwrap();
    assert_eq!(state2["count"], 30);

    // Current state should still be 30
    let current = manager.snapshot().await;
    assert_eq!(current["count"], 30);
}

#[tokio::test]
async fn test_manager_replay_invalid_index() {
    let manager = StateManager::new(json!({}));

    // No patches applied yet
    let result = manager.replay_to(0).await;
    assert!(result.is_err());

    manager
        .commit(make_patch(vec![Op::set(path!("x"), json!(1))], "s1"))
        .await
        .unwrap();

    // Index 1 doesn't exist (only 0 exists)
    let result = manager.replay_to(1).await;
    assert!(result.is_err());

    // Index 0 should work
    let state = manager.replay_to(0).await.unwrap();
    assert_eq!(state["x"], 1);
}

#[tokio::test]
async fn test_manager_replay_complex_state() {
    let manager = StateManager::new(json!({}));

    // Build up complex state over time
    manager
        .commit(make_patch(
            vec![
                Op::set(path!("user", "name"), json!("Alice")),
                Op::set(path!("user", "age"), json!(25)),
            ],
            "create_user",
        ))
        .await
        .unwrap();

    manager
        .commit(make_patch(
            vec![Op::set(path!("user", "email"), json!("alice@example.com"))],
            "add_email",
        ))
        .await
        .unwrap();

    manager
        .commit(make_patch(
            vec![Op::Increment {
                path: path!("user", "age"),
                amount: carve_state::Number::Int(1),
            }],
            "birthday",
        ))
        .await
        .unwrap();

    // Replay to different points
    let after_create = manager.replay_to(0).await.unwrap();
    assert_eq!(after_create["user"]["name"], "Alice");
    assert_eq!(after_create["user"]["age"], 25);
    assert!(after_create["user"].get("email").is_none());

    let after_email = manager.replay_to(1).await.unwrap();
    assert_eq!(after_email["user"]["email"], "alice@example.com");
    assert_eq!(after_email["user"]["age"], 25);

    let after_birthday = manager.replay_to(2).await.unwrap();
    assert_eq!(after_birthday["user"]["age"], 26);
}

// ============================================================================
// Clear history tests
// ============================================================================

#[tokio::test]
async fn test_manager_clear_history() {
    let manager = StateManager::new(json!({}));

    manager
        .commit(make_patch(vec![Op::set(path!("x"), json!(100))], "s1"))
        .await
        .unwrap();

    manager
        .commit(make_patch(vec![Op::set(path!("y"), json!(200))], "s2"))
        .await
        .unwrap();

    assert_eq!(manager.history_len().await, 2);

    manager.clear_history().await;

    assert_eq!(manager.history_len().await, 0);

    // State should be preserved
    let snapshot = manager.snapshot().await;
    assert_eq!(snapshot["x"], 100);
    assert_eq!(snapshot["y"], 200);

    // Replay should fail now
    let result = manager.replay_to(0).await;
    assert!(result.is_err());
}

// ============================================================================
// Clone behavior tests
// ============================================================================

#[tokio::test]
async fn test_manager_clone_shares_state() {
    let manager1 = StateManager::new(json!({"value": 0}));
    let manager2 = manager1.clone();

    // Changes through manager1 visible in manager2
    manager1
        .commit(make_patch(vec![Op::set(path!("value"), json!(42))], "s1"))
        .await
        .unwrap();

    let snapshot = manager2.snapshot().await;
    assert_eq!(snapshot["value"], 42);

    // Changes through manager2 visible in manager1
    manager2
        .commit(make_patch(vec![Op::set(path!("value"), json!(100))], "s2"))
        .await
        .unwrap();

    let snapshot = manager1.snapshot().await;
    assert_eq!(snapshot["value"], 100);

    // Both see same history
    assert_eq!(manager1.history_len().await, 2);
    assert_eq!(manager2.history_len().await, 2);
}

// ============================================================================
// Integration with Context tests
// ============================================================================

#[tokio::test]
async fn test_manager_with_context_workflow() {
    let manager = StateManager::new(json!({
        "counters": {
            "main": {"value": 0, "label": "Main"}
        }
    }));

    // Simulate tool execution workflow
    for i in 1..=3 {
        let snapshot = manager.snapshot().await;
        let ctx = Context::new(&snapshot, format!("call_{}", i), format!("tool:increment"));

        let counter = ctx.state::<CounterState>("counters.main");
        let current = counter.value().unwrap();
        counter.set_value(current + 10);
        counter.set_label(format!("After step {}", i));

        let tracked_patch = ctx.take_patch();
        manager.commit(tracked_patch).await.unwrap();
    }

    // Verify final state
    let final_state = manager.snapshot().await;
    assert_eq!(final_state["counters"]["main"]["value"], 30);
    assert_eq!(final_state["counters"]["main"]["label"], "After step 3");

    // Verify history
    assert_eq!(manager.history_len().await, 3);

    // Replay to middle step
    let mid_state = manager.replay_to(1).await.unwrap();
    assert_eq!(mid_state["counters"]["main"]["value"], 20);
}

#[tokio::test]
async fn test_manager_deterministic_replay() {
    let manager = StateManager::new(json!({"count": 0}));

    // Apply same sequence of operations
    let operations = vec![
        Op::set(path!("count"), json!(10)),
        Op::Increment {
            path: path!("count"),
            amount: carve_state::Number::Int(5),
        },
        Op::Decrement {
            path: path!("count"),
            amount: carve_state::Number::Int(3),
        },
        Op::set(path!("count"), json!(100)),
    ];

    for (i, op) in operations.into_iter().enumerate() {
        let patch = make_patch(vec![op], &format!("op_{}", i));
        manager.commit(patch).await.unwrap();
    }

    // Replay should always produce same results
    for _ in 0..3 {
        let state0 = manager.replay_to(0).await.unwrap();
        assert_eq!(state0["count"], 10);

        let state1 = manager.replay_to(1).await.unwrap();
        assert_eq!(state1["count"], 15);

        let state2 = manager.replay_to(2).await.unwrap();
        assert_eq!(state2["count"], 12);

        let state3 = manager.replay_to(3).await.unwrap();
        assert_eq!(state3["count"], 100);
    }
}

// ============================================================================
// Error handling tests
// ============================================================================

#[tokio::test]
async fn test_manager_commit_invalid_op() {
    let manager = StateManager::new(json!({"value": "not a number"}));

    // Try to increment a string (should fail)
    let patch = make_patch(
        vec![Op::Increment {
            path: path!("value"),
            amount: carve_state::Number::Int(1),
        }],
        "bad_op",
    );

    let result = manager.commit(patch).await;
    assert!(result.is_err());
}

// ============================================================================
// Concurrent access tests (basic)
// ============================================================================

#[tokio::test]
async fn test_manager_concurrent_reads() {
    let manager = StateManager::new(json!({"value": 42}));

    // Spawn multiple concurrent reads
    let handles: Vec<_> = (0..10)
        .map(|_| {
            let m = manager.clone();
            tokio::spawn(async move { m.snapshot().await })
        })
        .collect();

    // All should return same value
    for handle in handles {
        let snapshot = handle.await.unwrap();
        assert_eq!(snapshot["value"], 42);
    }
}

// ============================================================================
// Patch tracking tests
// ============================================================================

#[tokio::test]
async fn test_manager_tracks_patch_source() {
    let manager = StateManager::new(json!({}));

    manager
        .commit(make_patch(
            vec![Op::set(path!("a"), json!(1))],
            "tool:create",
        ))
        .await
        .unwrap();

    manager
        .commit(make_patch(
            vec![Op::set(path!("b"), json!(2))],
            "tool:update",
        ))
        .await
        .unwrap();

    manager
        .commit(make_patch(
            vec![Op::set(path!("c"), json!(3))],
            "system:auto",
        ))
        .await
        .unwrap();

    let history = manager.history().await;
    assert_eq!(history[0].source.as_deref(), Some("tool:create"));
    assert_eq!(history[1].source.as_deref(), Some("tool:update"));
    assert_eq!(history[2].source.as_deref(), Some("system:auto"));
}

// ============================================================================
// Preview patch tests
// ============================================================================

#[tokio::test]
async fn test_manager_preview_patch() {
    let manager = StateManager::new(json!({"count": 10}));

    let patch = Patch::new().with_op(Op::set(path!("count"), json!(100)));

    // Preview should show what would happen
    let preview = manager.preview_patch(&patch).await.unwrap();
    assert_eq!(preview["count"], 100);

    // But state should be unchanged
    let snapshot = manager.snapshot().await;
    assert_eq!(snapshot["count"], 10);

    // History should be empty
    assert_eq!(manager.history_len().await, 0);
}

#[tokio::test]
async fn test_manager_preview_patch_complex() {
    let manager = StateManager::new(json!({
        "user": {"name": "Alice", "age": 25}
    }));

    let patch = Patch::new()
        .with_op(Op::set(path!("user", "age"), json!(26)))
        .with_op(Op::set(path!("user", "email"), json!("alice@test.com")));

    let preview = manager.preview_patch(&patch).await.unwrap();
    assert_eq!(preview["user"]["age"], 26);
    assert_eq!(preview["user"]["email"], "alice@test.com");
    assert_eq!(preview["user"]["name"], "Alice"); // Unchanged fields preserved

    // Original state unchanged
    let snapshot = manager.snapshot().await;
    assert_eq!(snapshot["user"]["age"], 25);
    assert!(snapshot["user"].get("email").is_none());
}

// ============================================================================
// apply_patches pure function tests
// ============================================================================

#[test]
fn test_apply_patches_basic() {
    use carve_state::apply_patches;

    let doc = json!({"count": 0});
    let patches = vec![
        Patch::new().with_op(Op::set(path!("count"), json!(1))),
        Patch::new().with_op(Op::set(path!("count"), json!(2))),
        Patch::new().with_op(Op::set(path!("count"), json!(3))),
    ];

    let result = apply_patches(&doc, patches.iter()).unwrap();
    assert_eq!(result["count"], 3);

    // Original unchanged
    assert_eq!(doc["count"], 0);
}

#[test]
fn test_apply_patches_empty() {
    use carve_state::apply_patches;

    let doc = json!({"value": 42});
    let patches: Vec<Patch> = vec![];

    let result = apply_patches(&doc, patches.iter()).unwrap();
    assert_eq!(result, doc);
}

#[test]
fn test_apply_patches_error_stops_early() {
    use carve_state::apply_patches;

    let doc = json!({"count": "not a number"});
    let patches = vec![
        Patch::new().with_op(Op::set(path!("valid"), json!(1))), // This will work
        Patch::new().with_op(Op::Increment {
            // This will fail
            path: path!("count"),
            amount: carve_state::Number::Int(1),
        }),
        Patch::new().with_op(Op::set(path!("never_reached"), json!(true))),
    ];

    let result = apply_patches(&doc, patches.iter());
    assert!(result.is_err());
}
