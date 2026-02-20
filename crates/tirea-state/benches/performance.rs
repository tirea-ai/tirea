//! Performance benchmarks for tirea-state operations.
//!
//! Run with: cargo bench --package tirea-state

use tirea_state::{apply_patch, path, Op, Patch};
use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use serde_json::{json, Value};

// ============================================================================
// Helper functions to generate test data
// ============================================================================

/// Generate a flat document with N fields
fn generate_flat_doc(num_fields: usize) -> Value {
    let mut obj = serde_json::Map::new();
    for i in 0..num_fields {
        obj.insert(format!("field_{}", i), json!(i));
    }
    json!(obj)
}

/// Generate a deeply nested document
fn generate_nested_doc(depth: usize) -> Value {
    let mut current = json!({"value": 42});
    for i in (0..depth).rev() {
        let mut obj = serde_json::Map::new();
        obj.insert(format!("level_{}", i), current);
        current = json!(obj);
    }
    current
}

/// Generate a patch with N set operations
fn generate_set_patch(num_ops: usize) -> Patch {
    let mut patch = Patch::new();
    for i in 0..num_ops {
        patch.push(Op::set(path!(format!("field_{}", i)), json!(i * 2)));
    }
    patch
}

/// Generate a patch with redundant operations (for canonicalize testing)
fn generate_redundant_patch(num_ops: usize) -> Patch {
    let mut patch = Patch::new();
    // Create many operations on the same paths
    for round in 0..num_ops {
        for i in 0..10 {
            patch.push(Op::set(path!(format!("field_{}", i)), json!(round)));
        }
    }
    patch
}

// ============================================================================
// Benchmark: apply_patch with varying document sizes
// ============================================================================

fn bench_apply_patch_flat(c: &mut Criterion) {
    let mut group = c.benchmark_group("apply_patch_flat_doc");

    for num_fields in [10, 100, 1000, 10000] {
        group.throughput(Throughput::Elements(num_fields as u64));

        let doc = generate_flat_doc(num_fields);
        let patch = generate_set_patch(num_fields / 10); // 10% of fields modified

        group.bench_with_input(
            BenchmarkId::from_parameter(num_fields),
            &num_fields,
            |b, _| {
                b.iter(|| {
                    let result = apply_patch(black_box(&doc), black_box(&patch));
                    black_box(result)
                });
            },
        );
    }

    group.finish();
}

// ============================================================================
// Benchmark: apply_patch with deep nesting
// ============================================================================

fn bench_apply_patch_nested(c: &mut Criterion) {
    let mut group = c.benchmark_group("apply_patch_nested_doc");

    for depth in [5, 10, 20, 50] {
        let doc = generate_nested_doc(depth);

        // Create a patch that modifies the deepest value
        let mut path_segments = vec![];
        for i in 0..depth {
            path_segments.push(format!("level_{}", i));
        }
        path_segments.push("value".to_string());

        let patch = Patch::new().with_op(Op::set(
            tirea_state::Path::from_segments(
                path_segments
                    .iter()
                    .map(|s| tirea_state::Seg::Key(s.clone()))
                    .collect(),
            ),
            json!(999),
        ));

        group.bench_with_input(BenchmarkId::from_parameter(depth), &depth, |b, _| {
            b.iter(|| {
                let result = apply_patch(black_box(&doc), black_box(&patch));
                black_box(result)
            });
        });
    }

    group.finish();
}

// ============================================================================
// Benchmark: canonicalize performance
// ============================================================================

fn bench_canonicalize(c: &mut Criterion) {
    let mut group = c.benchmark_group("patch_canonicalize");

    for num_rounds in [10, 50, 100, 500] {
        let patch = generate_redundant_patch(num_rounds);
        let total_ops = patch.len();

        group.throughput(Throughput::Elements(total_ops as u64));

        group.bench_with_input(
            BenchmarkId::from_parameter(total_ops),
            &total_ops,
            |b, _| {
                b.iter(|| {
                    let canonical = black_box(&patch).canonicalize();
                    black_box(canonical)
                });
            },
        );
    }

    group.finish();
}

// ============================================================================
// Benchmark: Different operation types
// ============================================================================

fn bench_operation_types(c: &mut Criterion) {
    let mut group = c.benchmark_group("operation_types");

    let doc = json!({
        "counter": 0,
        "items": [1, 2, 3],
        "user": {"name": "Alice", "age": 30}
    });

    // Set operation
    group.bench_function("set", |b| {
        let patch = Patch::new().with_op(Op::set(path!("counter"), json!(42)));
        b.iter(|| {
            let result = apply_patch(black_box(&doc), black_box(&patch));
            black_box(result)
        });
    });

    // Increment operation
    group.bench_function("increment", |b| {
        let patch = Patch::new().with_op(Op::increment(path!("counter"), 1i64));
        b.iter(|| {
            let result = apply_patch(black_box(&doc), black_box(&patch));
            black_box(result)
        });
    });

    // Append operation
    group.bench_function("append", |b| {
        let patch = Patch::new().with_op(Op::append(path!("items"), json!(4)));
        b.iter(|| {
            let result = apply_patch(black_box(&doc), black_box(&patch));
            black_box(result)
        });
    });

    // MergeObject operation
    group.bench_function("merge_object", |b| {
        let patch = Patch::new().with_op(Op::merge_object(path!("user"), json!({"city": "NYC"})));
        b.iter(|| {
            let result = apply_patch(black_box(&doc), black_box(&patch));
            black_box(result)
        });
    });

    group.finish();
}

// ============================================================================
// Benchmark: Multiple operations in sequence
// ============================================================================

fn bench_multiple_operations(c: &mut Criterion) {
    let mut group = c.benchmark_group("multiple_operations");

    let doc = generate_flat_doc(1000);

    for num_ops in [10, 50, 100, 500] {
        let patch = generate_set_patch(num_ops);

        group.throughput(Throughput::Elements(num_ops as u64));

        group.bench_with_input(BenchmarkId::from_parameter(num_ops), &num_ops, |b, _| {
            b.iter(|| {
                let result = apply_patch(black_box(&doc), black_box(&patch));
                black_box(result)
            });
        });
    }

    group.finish();
}

// ============================================================================
// Benchmark: Canonicalize effectiveness
// ============================================================================

fn bench_canonicalize_effectiveness(c: &mut Criterion) {
    let mut group = c.benchmark_group("canonicalize_effectiveness");

    let doc = generate_flat_doc(100);

    // Create a patch with many redundant operations
    let redundant_patch = generate_redundant_patch(100);
    let canonical_patch = redundant_patch.canonicalize();

    group.bench_function("with_redundant_ops", |b| {
        b.iter(|| {
            let result = apply_patch(black_box(&doc), black_box(&redundant_patch));
            black_box(result)
        });
    });

    group.bench_function("after_canonicalize", |b| {
        b.iter(|| {
            let result = apply_patch(black_box(&doc), black_box(&canonical_patch));
            black_box(result)
        });
    });

    // Show the reduction
    eprintln!(
        "Canonicalize reduced ops from {} to {} ({:.1}% reduction)",
        redundant_patch.len(),
        canonical_patch.len(),
        100.0 * (1.0 - canonical_patch.len() as f64 / redundant_patch.len() as f64)
    );

    group.finish();
}

criterion_group!(
    benches,
    bench_apply_patch_flat,
    bench_apply_patch_nested,
    bench_canonicalize,
    bench_operation_types,
    bench_multiple_operations,
    bench_canonicalize_effectiveness,
);

criterion_main!(benches);
