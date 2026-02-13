//! Benchmarks for Session/Thread operations including state rebuilding and patch application.
//!
//! Run with: cargo bench --package carve-agent --bench session_operations

use carve_agent::session::Session;
use carve_agent::types::Message;
use carve_state::{path, Op, Patch, TrackedPatch};
use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use serde_json::json;

// ============================================================================
// Helper functions
// ============================================================================

/// Generate a realistic state document
fn generate_state(size: &str) -> serde_json::Value {
    match size {
        "small" => json!({
            "counter": 0,
            "user": {
                "name": "Alice",
                "preferences": {
                    "theme": "dark",
                    "language": "en"
                }
            },
            "items": [1, 2, 3]
        }),
        "medium" => {
            let mut obj = serde_json::Map::new();
            obj.insert("session_id".to_string(), json!("sess-123"));
            obj.insert(
                "user".to_string(),
                json!({
                    "id": "user-456",
                    "name": "Bob",
                    "email": "bob@example.com",
                    "metadata": {
                        "created_at": 1234567890,
                        "last_login": 1234567999,
                        "preferences": {
                            "theme": "light",
                            "language": "en",
                            "notifications": true
                        }
                    }
                }),
            );
            obj.insert(
                "tools".to_string(),
                json!({
                    "search": {"enabled": true, "calls": 0},
                    "calculator": {"enabled": true, "calls": 0},
                    "weather": {"enabled": false, "calls": 0}
                }),
            );
            obj.insert("history".to_string(), json!([]));
            json!(obj)
        }
        "large" => {
            let mut obj = serde_json::Map::new();
            // Add many fields
            for i in 0..100 {
                obj.insert(
                    format!("field_{}", i),
                    json!({
                        "value": i,
                        "metadata": {
                            "created": 1234567890,
                            "modified": 1234567899
                        }
                    }),
                );
            }
            json!(obj)
        }
        _ => json!({}),
    }
}

/// Generate N patches with various operations
fn generate_patches(count: usize) -> Vec<TrackedPatch> {
    let mut patches = Vec::new();

    for i in 0..count {
        let mut patch = Patch::new();

        match i % 4 {
            0 => {
                // Set operation
                patch.push(Op::set(path!("counter"), json!(i)));
            }
            1 => {
                // Nested set
                patch.push(Op::set(path!("user", "name"), json!(format!("User{}", i))));
            }
            2 => {
                // Increment
                patch.push(Op::increment(path!("counter"), 1i64));
            }
            3 => {
                // Append to array
                patch.push(Op::append(path!("items"), json!(i)));
            }
            _ => {}
        }

        patches.push(TrackedPatch::new(patch));
    }

    patches
}

/// Generate messages for conversation
fn generate_messages(count: usize) -> Vec<Message> {
    let mut messages = Vec::new();
    for i in 0..count {
        if i % 2 == 0 {
            messages.push(Message::user(format!("Question {}", i)));
        } else {
            messages.push(Message::assistant(format!("Answer {}", i)));
        }
    }
    messages
}

// ============================================================================
// Benchmark: State cloning
// ============================================================================

fn bench_state_clone(c: &mut Criterion) {
    let mut group = c.benchmark_group("state_clone");

    for size in ["small", "medium", "large"] {
        let state = generate_state(size);
        let state_size = serde_json::to_vec(&state).unwrap().len();

        group.throughput(Throughput::Bytes(state_size as u64));

        group.bench_with_input(BenchmarkId::new(size, state_size), &state, |b, s| {
            b.iter(|| {
                let cloned = black_box(s).clone();
                black_box(cloned)
            });
        });
    }

    group.finish();
}

// ============================================================================
// Benchmark: Session rebuild_state
// ============================================================================

fn bench_session_rebuild(c: &mut Criterion) {
    let mut group = c.benchmark_group("session_rebuild_state");

    for patch_count in [0, 10, 50, 100, 500] {
        let state = generate_state("medium");
        let patches = generate_patches(patch_count);

        let mut session = Session::with_initial_state("test", state);
        for patch in patches {
            session = session.with_patch(patch);
        }

        group.throughput(Throughput::Elements(patch_count as u64));

        group.bench_with_input(
            BenchmarkId::from_parameter(patch_count),
            &session,
            |b, sess| {
                b.iter(|| {
                    let rebuilt = black_box(sess).rebuild_state();
                    black_box(rebuilt)
                });
            },
        );
    }

    group.finish();
}

// ============================================================================
// Benchmark: Session replay_to (time-travel)
// ============================================================================

fn bench_session_replay(c: &mut Criterion) {
    let mut group = c.benchmark_group("session_replay_to");

    let patch_count = 100;
    let state = generate_state("medium");
    let patches = generate_patches(patch_count);

    let mut session = Session::with_initial_state("test", state);
    for patch in patches {
        session = session.with_patch(patch);
    }

    // Replay to different points in history
    for target_index in [0, 25, 50, 75, 99] {
        group.bench_with_input(
            BenchmarkId::new("replay_to", target_index),
            &target_index,
            |b, &idx| {
                b.iter(|| {
                    let replayed = black_box(&session).replay_to(idx);
                    black_box(replayed)
                });
            },
        );
    }

    group.finish();
}

// ============================================================================
// Benchmark: Session snapshot
// ============================================================================

fn bench_session_snapshot(c: &mut Criterion) {
    let mut group = c.benchmark_group("session_snapshot");

    for patch_count in [10, 50, 100, 500] {
        let state = generate_state("medium");
        let patches = generate_patches(patch_count);

        let mut session = Session::with_initial_state("test", state);
        for patch in patches {
            session = session.with_patch(patch);
        }

        group.throughput(Throughput::Elements(patch_count as u64));

        group.bench_with_input(
            BenchmarkId::from_parameter(patch_count),
            &session,
            |b, sess| {
                b.iter(|| {
                    let snapshotted = black_box(sess).clone().snapshot();
                    black_box(snapshotted)
                });
            },
        );
    }

    group.finish();
}

// ============================================================================
// Benchmark: Session with_message (builder pattern)
// ============================================================================

fn bench_session_with_message(c: &mut Criterion) {
    let mut group = c.benchmark_group("session_with_message");

    for msg_count in [10, 50, 100, 200] {
        let messages = generate_messages(msg_count);

        group.throughput(Throughput::Elements(msg_count as u64));

        group.bench_with_input(
            BenchmarkId::from_parameter(msg_count),
            &messages,
            |b, msgs| {
                b.iter(|| {
                    let mut session = Session::new("test");
                    for msg in msgs {
                        session = session.with_message(msg.clone());
                    }
                    black_box(session)
                });
            },
        );
    }

    group.finish();
}

// ============================================================================
// Benchmark: Full session clone (messages + state + patches)
// ============================================================================

fn bench_full_session_clone(c: &mut Criterion) {
    let mut group = c.benchmark_group("full_session_clone");

    for scenario in ["short", "medium", "long"] {
        let (msg_count, patch_count) = match scenario {
            "short" => (20, 10),
            "medium" => (100, 50),
            "long" => (500, 200),
            _ => (0, 0),
        };

        let messages = generate_messages(msg_count);
        let patches = generate_patches(patch_count);
        let state = generate_state("medium");

        let mut session = Session::with_initial_state("test", state);
        for msg in messages {
            session = session.with_message(msg);
        }
        for patch in patches {
            session = session.with_patch(patch);
        }

        group.throughput(Throughput::Elements((msg_count + patch_count) as u64));

        group.bench_with_input(
            BenchmarkId::new(scenario, msg_count),
            &session,
            |b, sess| {
                b.iter(|| {
                    let cloned = black_box(sess).clone();
                    black_box(cloned)
                });
            },
        );
    }

    group.finish();
}

// ============================================================================
// Benchmark: Simulating agent loop (repeated clone + rebuild)
// ============================================================================

fn bench_agent_loop_simulation(c: &mut Criterion) {
    let mut group = c.benchmark_group("agent_loop_simulation");

    let turns = 10;

    for initial_msgs in [20, 50, 100] {
        group.bench_with_input(
            BenchmarkId::new("loop_turns", initial_msgs),
            &initial_msgs,
            |b, &msg_count| {
                b.iter(|| {
                    let mut session = Session::with_initial_state("test", generate_state("medium"));

                    // Add initial messages
                    for msg in generate_messages(msg_count) {
                        session = session.with_message(msg);
                    }

                    // Simulate agent loop
                    for i in 0..turns {
                        // 1. Clone session for processing (happens in agent loop)
                        let _working_copy = session.clone();

                        // 2. Rebuild state (happens before LLM call)
                        let _current_state = session.rebuild_state().unwrap();

                        // 3. Add new message and patch (simulating tool execution)
                        session = session.with_message(Message::assistant(format!("Turn {}", i)));
                        session = session.with_patch(TrackedPatch::new(
                            Patch::new().with_op(Op::increment(path!("counter"), 1i64)),
                        ));
                    }

                    black_box(session)
                });
            },
        );
    }

    group.finish();
}

criterion_group!(
    benches,
    bench_state_clone,
    bench_session_rebuild,
    bench_session_replay,
    bench_session_snapshot,
    bench_session_with_message,
    bench_full_session_clone,
    bench_agent_loop_simulation,
);

criterion_main!(benches);
