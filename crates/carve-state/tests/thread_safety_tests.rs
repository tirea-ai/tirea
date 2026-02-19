//! Thread safety tests for carve-state.
//!
//! These tests verify that the API is thread-safe and works correctly
//! in concurrent scenarios.

use carve_state::{path, DocCell, Op, Patch, PatchSink, StateContext, StateManager, TrackedPatch};
use carve_state_derive::State;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::sync::{Arc, Mutex};
use std::thread;

// ============================================================================
// Test state types
// ============================================================================

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, State)]
struct CounterState {
    value: i64,
    label: String,
}

// ============================================================================
// PatchSink thread safety tests
// ============================================================================

#[test]
fn test_patch_sink_thread_safe() {
    let ops = Arc::new(Mutex::new(Vec::new()));

    // Create multiple threads that collect operations
    let handles: Vec<_> = (0..10)
        .map(|i| {
            let ops_clone = Arc::clone(&ops);
            thread::spawn(move || {
                let sink = PatchSink::new(&ops_clone);
                sink.collect(Op::set(path!("field"), json!(i)));
            })
        })
        .collect();

    // Wait for all threads
    for handle in handles {
        handle.join().unwrap();
    }

    // Verify all operations were collected
    let collected = ops.lock().unwrap();
    assert_eq!(collected.len(), 10);
}

#[test]
fn test_patch_sink_concurrent_writes() {
    let ops = Arc::new(Mutex::new(Vec::new()));

    // Spawn many threads doing rapid writes
    let handles: Vec<_> = (0..100)
        .map(|i| {
            let ops_clone = Arc::clone(&ops);
            thread::spawn(move || {
                let sink = PatchSink::new(&ops_clone);
                // Each thread writes multiple operations
                for j in 0..10 {
                    sink.collect(Op::set(path!("data"), json!(i * 10 + j)));
                }
            })
        })
        .collect();

    for handle in handles {
        handle.join().unwrap();
    }

    let collected = ops.lock().unwrap();
    assert_eq!(collected.len(), 1000);
}

// ============================================================================
// StateRef thread safety tests (compile-time verification)
// ============================================================================

fn assert_send<T: Send>() {}
fn assert_sync<T: Sync>() {}

#[test]
fn test_state_ref_is_send() {
    // StateRef should be Send
    assert_send::<CounterStateRef<'_>>();
}

#[test]
fn test_context_is_send() {
    // StateContext should be Send
    assert_send::<StateContext<'_>>();
}

#[test]
fn test_context_is_sync() {
    // StateContext should be Sync (for sharing across threads)
    assert_sync::<StateContext<'_>>();
}

// ============================================================================
// Context concurrent access tests
// ============================================================================

#[test]
fn test_context_concurrent_state_access() {
    let doc = json!({
        "counter1": {"value": 0, "label": "c1"},
        "counter2": {"value": 0, "label": "c2"},
        "counter3": {"value": 0, "label": "c3"}
    });

    // Create context
    let doc_cell = DocCell::new(doc.clone());
    let ctx = StateContext::new(&doc_cell);

    // Access state from main thread
    let c1 = ctx.state::<CounterState>("counter1");
    c1.set_value(10);

    let c2 = ctx.state::<CounterState>("counter2");
    c2.set_value(20);

    let c3 = ctx.state::<CounterState>("counter3");
    c3.set_value(30);

    // Verify all operations collected
    assert_eq!(ctx.ops_count(), 3);
}

#[test]
fn test_context_scoped_parallel_access() {
    let doc = json!({
        "counters": {
            "a": {"value": 0, "label": "a"},
            "b": {"value": 0, "label": "b"},
            "c": {"value": 0, "label": "c"}
        }
    });

    let doc_cell = DocCell::new(doc.clone());
    let ctx = Arc::new(StateContext::new(&doc_cell));

    // Use scoped threads to access context
    thread::scope(|s| {
        for i in 0..3 {
            let ctx_ref = Arc::clone(&ctx);
            s.spawn(move || {
                let path = format!("counters.{}", ['a', 'b', 'c'][i]);
                let counter = ctx_ref.state::<CounterState>(&path);
                counter.set_value(i as i64 * 10);
            });
        }
    });

    assert_eq!(ctx.ops_count(), 3);
}

// ============================================================================
// StateManager async concurrency tests
// ============================================================================

fn make_patch(ops: Vec<Op>, source: &str) -> TrackedPatch {
    TrackedPatch::new(Patch::with_ops(ops)).with_source(source)
}

#[tokio::test]
async fn test_manager_concurrent_reads() {
    let manager = StateManager::new(json!({"value": 42}));

    // Spawn multiple concurrent reads
    let handles: Vec<_> = (0..100)
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

#[tokio::test]
async fn test_manager_sequential_writes() {
    let manager = StateManager::new(json!({"count": 0}));

    // Sequential writes should be deterministic
    for i in 1..=100 {
        let patch = make_patch(
            vec![Op::set(path!("count"), json!(i))],
            &format!("write_{}", i),
        );
        manager.commit(patch).await.unwrap();
    }

    let snapshot = manager.snapshot().await;
    assert_eq!(snapshot["count"], 100);
    assert_eq!(manager.history_len().await, 100);
}

#[tokio::test]
async fn test_manager_concurrent_writes_serialized() {
    let manager = StateManager::new(json!({"count": 0}));

    // Note: Concurrent writes to StateManager are serialized by the RwLock
    // The final order depends on scheduling, but all writes will be applied
    let handles: Vec<_> = (0..10)
        .map(|i| {
            let m = manager.clone();
            tokio::spawn(async move {
                let patch = make_patch(vec![Op::set(path!("count"), json!(i))], &format!("w{}", i));
                m.commit(patch).await.unwrap();
            })
        })
        .collect();

    for handle in handles {
        handle.await.unwrap();
    }

    // All writes applied
    assert_eq!(manager.history_len().await, 10);
}

#[tokio::test]
async fn test_manager_read_write_concurrency() {
    let manager = StateManager::new(json!({"value": 0}));

    // Writer task
    let writer_manager = manager.clone();
    let writer = tokio::spawn(async move {
        for i in 1..=50 {
            let patch = make_patch(vec![Op::set(path!("value"), json!(i))], &format!("w{}", i));
            writer_manager.commit(patch).await.unwrap();
            tokio::task::yield_now().await;
        }
    });

    // Reader tasks
    let readers: Vec<_> = (0..5)
        .map(|_| {
            let m = manager.clone();
            tokio::spawn(async move {
                for _ in 0..20 {
                    let snapshot = m.snapshot().await;
                    // Value should be between 0 and 50
                    let val = snapshot["value"].as_i64().unwrap();
                    assert!((0..=50).contains(&val));
                    tokio::task::yield_now().await;
                }
            })
        })
        .collect();

    writer.await.unwrap();
    for reader in readers {
        reader.await.unwrap();
    }

    // Final value should be 50
    let final_snapshot = manager.snapshot().await;
    assert_eq!(final_snapshot["value"], 50);
}

#[tokio::test]
async fn test_manager_clone_concurrent_access() {
    let manager = StateManager::new(json!({"shared": 0}));

    // Create multiple clones
    let clones: Vec<_> = (0..5).map(|_| manager.clone()).collect();

    // Each clone writes
    let handles: Vec<_> = clones
        .into_iter()
        .enumerate()
        .map(|(i, m)| {
            tokio::spawn(async move {
                for j in 0..10 {
                    let patch = make_patch(
                        vec![Op::set(path!("shared"), json!(i * 10 + j))],
                        &format!("clone{}_{}", i, j),
                    );
                    m.commit(patch).await.unwrap();
                }
            })
        })
        .collect();

    for handle in handles {
        handle.await.unwrap();
    }

    // All writes from all clones should be in history
    assert_eq!(manager.history_len().await, 50);
}

// ============================================================================
// Stress tests
// ============================================================================

#[test]
fn test_stress_patch_sink_high_volume() {
    let ops = Arc::new(Mutex::new(Vec::new()));
    let sink = PatchSink::new(&ops);

    // Collect a large number of operations
    for i in 0..10000 {
        sink.collect(Op::set(path!("field"), json!(i)));
    }

    let collected = ops.lock().unwrap();
    assert_eq!(collected.len(), 10000);
}

#[tokio::test]
async fn test_stress_manager_many_patches() {
    let manager = StateManager::new(json!({}));

    // Apply many small patches
    for i in 0..1000 {
        let patch = make_patch(vec![Op::set(path!("count"), json!(i))], &format!("p{}", i));
        manager.commit(patch).await.unwrap();
    }

    assert_eq!(manager.history_len().await, 1000);

    // Replay should work
    let mid_state = manager.replay_to(500).await.unwrap();
    assert_eq!(mid_state["count"], 500);
}

#[tokio::test]
async fn test_stress_batch_apply() {
    let manager = StateManager::new(json!({}));

    // Create a large batch
    let patches: Vec<_> = (0..100)
        .map(|i| {
            make_patch(
                vec![Op::set(path!(format!("key_{}", i)), json!(i))],
                &format!("p{}", i),
            )
        })
        .collect();

    let result = manager.commit_batch(patches).await.unwrap();
    assert_eq!(result.patches_applied, 100);

    let snapshot = manager.snapshot().await;
    assert_eq!(snapshot["key_0"], 0);
    assert_eq!(snapshot["key_99"], 99);
}

// ============================================================================
// Determinism tests
// ============================================================================

#[tokio::test]
async fn test_deterministic_replay() {
    let manager = StateManager::new(json!({}));

    // Apply known sequence of operations
    let operations = vec![
        Op::set(path!("a"), json!(1)),
        Op::set(path!("b"), json!(2)),
        Op::set(path!("a"), json!(10)),
        Op::Increment {
            path: path!("b"),
            amount: carve_state::Number::Int(3),
        },
        Op::Delete { path: path!("a") },
    ];

    for (i, op) in operations.into_iter().enumerate() {
        manager
            .commit(make_patch(vec![op], &format!("op{}", i)))
            .await
            .unwrap();
    }

    // Replay multiple times - should always get same results
    for _ in 0..10 {
        let s0 = manager.replay_to(0).await.unwrap();
        assert_eq!(s0["a"], 1);
        assert!(s0.get("b").is_none());

        let s1 = manager.replay_to(1).await.unwrap();
        assert_eq!(s1["a"], 1);
        assert_eq!(s1["b"], 2);

        let s2 = manager.replay_to(2).await.unwrap();
        assert_eq!(s2["a"], 10);
        assert_eq!(s2["b"], 2);

        let s3 = manager.replay_to(3).await.unwrap();
        assert_eq!(s3["a"], 10);
        assert_eq!(s3["b"], 5);

        let s4 = manager.replay_to(4).await.unwrap();
        assert!(s4.get("a").is_none());
        assert_eq!(s4["b"], 5);
    }
}

// ============================================================================
// Data race prevention tests
// ============================================================================

#[tokio::test]
async fn test_no_data_race_on_snapshot() {
    let manager = StateManager::new(json!({"list": []}));

    // Concurrent append operations
    let handles: Vec<_> = (0..10)
        .map(|i| {
            let m = manager.clone();
            tokio::spawn(async move {
                let patch = make_patch(
                    vec![Op::Append {
                        path: path!("list"),
                        value: json!(i),
                    }],
                    &format!("append{}", i),
                );
                m.commit(patch).await.unwrap();
            })
        })
        .collect();

    for handle in handles {
        handle.await.unwrap();
    }

    // Final list should have exactly 10 elements
    let snapshot = manager.snapshot().await;
    let list = snapshot["list"].as_array().unwrap();
    assert_eq!(list.len(), 10);
}
