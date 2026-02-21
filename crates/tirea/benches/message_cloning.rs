//! Benchmarks for message cloning overhead.
//!
//! Run with: cargo bench --package tirea --bench message_cloning

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use serde_json::json;
use std::sync::Arc;
use tirea::contracts::thread::{Message, ToolCall};

// ============================================================================
// Helper functions to generate test data
// ============================================================================

/// Generate N messages simulating a conversation
fn generate_messages(count: usize) -> Vec<Message> {
    let mut messages = Vec::new();

    for i in 0..count {
        if i % 3 == 0 {
            // User message
            messages.push(Message::user(format!("User message {}", i)));
        } else if i % 3 == 1 {
            // Assistant message with text
            messages.push(Message::assistant(format!(
                "This is a longer assistant response {} with multiple sentences. \
                It might contain explanations, code snippets, or detailed information. \
                The length varies but typically contains several hundred characters.",
                i
            )));
        } else {
            // Assistant message with tool calls
            let tool_calls = vec![
                ToolCall::new(
                    format!("call_{}", i),
                    "search",
                    json!({"query": "test query", "limit": 10}),
                ),
                ToolCall::new(
                    format!("call_{}b", i),
                    "calculate",
                    json!({"expression": "2 + 2 * 10"}),
                ),
            ];
            messages.push(Message::assistant_with_tool_calls(
                "Let me help you with that",
                tool_calls,
            ));
        }
    }

    messages
}

/// Generate messages with Arc wrapper
fn generate_arc_messages(count: usize) -> Vec<Arc<Message>> {
    generate_messages(count).into_iter().map(Arc::new).collect()
}

// ============================================================================
// Benchmark: Message cloning
// ============================================================================

fn bench_message_clone(c: &mut Criterion) {
    let mut group = c.benchmark_group("message_clone");

    for msg_count in [1, 10, 50, 100, 500] {
        let messages = generate_messages(msg_count);

        group.throughput(Throughput::Elements(msg_count as u64));

        group.bench_with_input(
            BenchmarkId::new("owned", msg_count),
            &messages,
            |b, msgs| {
                b.iter(|| {
                    let cloned: Vec<Message> = black_box(msgs).clone();
                    black_box(cloned)
                });
            },
        );
    }

    group.finish();
}

// ============================================================================
// Benchmark: Arc<Message> cloning
// ============================================================================

fn bench_arc_message_clone(c: &mut Criterion) {
    let mut group = c.benchmark_group("arc_message_clone");

    for msg_count in [1, 10, 50, 100, 500] {
        let messages = generate_arc_messages(msg_count);

        group.throughput(Throughput::Elements(msg_count as u64));

        group.bench_with_input(BenchmarkId::new("arc", msg_count), &messages, |b, msgs| {
            b.iter(|| {
                let cloned: Vec<Arc<Message>> = black_box(msgs).clone();
                black_box(cloned)
            });
        });
    }

    group.finish();
}

// ============================================================================
// Benchmark: Comparison - Owned vs Arc
// ============================================================================

fn bench_clone_comparison(c: &mut Criterion) {
    let mut group = c.benchmark_group("clone_comparison");

    for msg_count in [10, 50, 100, 200, 500] {
        let owned_messages = generate_messages(msg_count);
        let arc_messages = generate_arc_messages(msg_count);

        group.throughput(Throughput::Elements(msg_count as u64));

        // Benchmark owned clone
        group.bench_with_input(
            BenchmarkId::new("owned", msg_count),
            &owned_messages,
            |b, msgs| {
                b.iter(|| {
                    let cloned: Vec<Message> = black_box(msgs).clone();
                    black_box(cloned)
                });
            },
        );

        // Benchmark Arc clone
        group.bench_with_input(
            BenchmarkId::new("arc", msg_count),
            &arc_messages,
            |b, msgs| {
                b.iter(|| {
                    let cloned: Vec<Arc<Message>> = black_box(msgs).clone();
                    black_box(cloned)
                });
            },
        );
    }

    group.finish();
}

// ============================================================================
// Benchmark: Simulating LLM request building (clone entire history)
// ============================================================================

fn bench_llm_request_building(c: &mut Criterion) {
    let mut group = c.benchmark_group("llm_request_building");

    // Simulate different conversation lengths
    for conversation_length in [20, 50, 100, 200, 500] {
        let owned_messages = generate_messages(conversation_length);
        let arc_messages = generate_arc_messages(conversation_length);

        group.throughput(Throughput::Elements(conversation_length as u64));

        // Owned: simulate building request (clone all messages)
        group.bench_with_input(
            BenchmarkId::new("owned_build_request", conversation_length),
            &owned_messages,
            |b, msgs| {
                b.iter(|| {
                    let mut request_messages = Vec::new();
                    // System prompt
                    request_messages.push(Message::system("You are a helpful assistant"));
                    // Clone conversation history
                    request_messages.extend(black_box(msgs).clone());
                    black_box(request_messages)
                });
            },
        );

        // Arc: simulate building request (clone Arc pointers)
        group.bench_with_input(
            BenchmarkId::new("arc_build_request", conversation_length),
            &arc_messages,
            |b, msgs| {
                b.iter(|| {
                    let mut request_messages: Vec<Arc<Message>> = Vec::new();
                    // System prompt
                    request_messages.push(Arc::new(Message::system("You are a helpful assistant")));
                    // Clone Arc pointers
                    request_messages.extend(black_box(msgs).clone());
                    black_box(request_messages)
                });
            },
        );
    }

    group.finish();
}

// ============================================================================
// Benchmark: Memory allocation overhead
// ============================================================================

fn bench_message_creation(c: &mut Criterion) {
    let mut group = c.benchmark_group("message_creation");

    // Creating owned message
    group.bench_function("create_owned_user_msg", |b| {
        b.iter(|| {
            let msg = Message::user(black_box("Hello, how can you help me today?"));
            black_box(msg)
        });
    });

    // Creating Arc-wrapped message
    group.bench_function("create_arc_user_msg", |b| {
        b.iter(|| {
            let msg = Arc::new(Message::user(black_box(
                "Hello, how can you help me today?",
            )));
            black_box(msg)
        });
    });

    // Creating owned message with tool calls
    group.bench_function("create_owned_tool_msg", |b| {
        b.iter(|| {
            let tool_calls = vec![
                ToolCall::new("call_1", "search", json!({"query": "test"})),
                ToolCall::new("call_2", "calculate", json!({"expr": "2+2"})),
            ];
            let msg = Message::assistant_with_tool_calls("Processing...", tool_calls);
            black_box(msg)
        });
    });

    // Creating Arc-wrapped message with tool calls
    group.bench_function("create_arc_tool_msg", |b| {
        b.iter(|| {
            let tool_calls = vec![
                ToolCall::new("call_1", "search", json!({"query": "test"})),
                ToolCall::new("call_2", "calculate", json!({"expr": "2+2"})),
            ];
            let msg = Arc::new(Message::assistant_with_tool_calls(
                "Processing...",
                tool_calls,
            ));
            black_box(msg)
        });
    });

    group.finish();
}

// ============================================================================
// Benchmark: Repeated cloning (simulating multi-turn conversation)
// ============================================================================

fn bench_repeated_cloning(c: &mut Criterion) {
    let mut group = c.benchmark_group("repeated_cloning");

    // Simulate 10 turns in a conversation
    let turns = 10;

    for initial_length in [20, 50, 100] {
        // Owned version
        group.bench_with_input(
            BenchmarkId::new("owned_10_turns", initial_length),
            &initial_length,
            |b, &len| {
                b.iter(|| {
                    let mut messages = generate_messages(len);
                    for i in 0..turns {
                        // Simulate LLM request: clone all messages
                        let _request = messages.clone();
                        // Add new message
                        messages.push(Message::assistant(format!("Response {}", i)));
                    }
                    black_box(messages)
                });
            },
        );

        // Arc version
        group.bench_with_input(
            BenchmarkId::new("arc_10_turns", initial_length),
            &initial_length,
            |b, &len| {
                b.iter(|| {
                    let mut messages = generate_arc_messages(len);
                    for i in 0..turns {
                        // Simulate LLM request: clone Arc pointers
                        let _request = messages.clone();
                        // Add new message
                        messages.push(Arc::new(Message::assistant(format!("Response {}", i))));
                    }
                    black_box(messages)
                });
            },
        );
    }

    group.finish();
}

criterion_group!(
    benches,
    bench_message_clone,
    bench_arc_message_clone,
    bench_clone_comparison,
    bench_llm_request_building,
    bench_message_creation,
    bench_repeated_cloning,
);

criterion_main!(benches);
