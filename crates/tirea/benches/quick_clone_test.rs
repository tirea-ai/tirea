//! Quick micro-benchmark to measure clone overhead
//!
//! Run with: cargo bench --package tirea --bench quick_clone_test

use criterion::{black_box, criterion_group, criterion_main, Criterion};
use std::sync::Arc;
use tirea::contracts::thread::Message;

fn generate_messages(count: usize) -> Vec<Message> {
    (0..count)
        .map(|i| {
            if i % 2 == 0 {
                Message::user(format!("User message {}", i))
            } else {
                Message::assistant(format!("Assistant response {} with some content", i))
            }
        })
        .collect()
}

fn bench_quick(c: &mut Criterion) {
    // Small conversation (20 messages)
    let msgs_20 = generate_messages(20);
    let arc_msgs_20: Vec<Arc<Message>> = generate_messages(20).into_iter().map(Arc::new).collect();

    c.bench_function("clone_20_owned", |b| {
        b.iter(|| {
            let cloned: Vec<Message> = black_box(&msgs_20).clone();
            black_box(cloned)
        });
    });

    c.bench_function("clone_20_arc", |b| {
        b.iter(|| {
            let cloned: Vec<Arc<Message>> = black_box(&arc_msgs_20).clone();
            black_box(cloned)
        });
    });

    // Medium conversation (100 messages)
    let msgs_100 = generate_messages(100);
    let arc_msgs_100: Vec<Arc<Message>> =
        generate_messages(100).into_iter().map(Arc::new).collect();

    c.bench_function("clone_100_owned", |b| {
        b.iter(|| {
            let cloned: Vec<Message> = black_box(&msgs_100).clone();
            black_box(cloned)
        });
    });

    c.bench_function("clone_100_arc", |b| {
        b.iter(|| {
            let cloned: Vec<Arc<Message>> = black_box(&arc_msgs_100).clone();
            black_box(cloned)
        });
    });

    // Large conversation (500 messages)
    let msgs_500 = generate_messages(500);
    let arc_msgs_500: Vec<Arc<Message>> =
        generate_messages(500).into_iter().map(Arc::new).collect();

    c.bench_function("clone_500_owned", |b| {
        b.iter(|| {
            let cloned: Vec<Message> = black_box(&msgs_500).clone();
            black_box(cloned)
        });
    });

    c.bench_function("clone_500_arc", |b| {
        b.iter(|| {
            let cloned: Vec<Arc<Message>> = black_box(&arc_msgs_500).clone();
            black_box(cloned)
        });
    });
}

criterion_group!(benches, bench_quick);
criterion_main!(benches);
