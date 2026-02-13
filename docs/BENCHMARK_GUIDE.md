# Carve Agent Performance Benchmark Guide

This document describes how to run and interpret performance benchmarks for the Carve agent framework.

## Quick Start

```bash
# Run all benchmarks for carve-agent
cargo bench --package carve-agent

# Run specific benchmark suites
cargo bench --package carve-agent --bench message_cloning
cargo bench --package carve-agent --bench session_operations
cargo bench --package carve-agent --bench quick_clone_test

# Run with quick mode (fewer samples, faster)
cargo bench --package carve-agent --bench quick_clone_test -- --quick

# Run carve-state benchmarks
cargo bench --package carve-state
```

## Benchmark Suites

### 1. `message_cloning.rs` - Message Clone Performance

**What it measures**:
- Message cloning overhead (owned vs Arc)
- LLM request building simulation
- Repeated cloning in multi-turn conversations
- Memory allocation overhead

**Key metrics**:
- Clone time per conversation length (20, 50, 100, 500 messages)
- Speedup factor (Arc vs owned)
- Throughput (messages/sec)

**Run time**: ~2-3 minutes (full mode), ~30 seconds (quick mode)

---

### 2. `session_operations.rs` - Session/Thread Operations

**What it measures**:
- State cloning (small, medium, large JSON)
- `rebuild_state()` performance with varying patch counts
- `replay_to()` time-travel debugging overhead
- `snapshot()` operation cost
- Full session clone (messages + state + patches)
- Agent loop simulation (repeated clone + rebuild)

**Key metrics**:
- Clone time vs state size
- Patch application throughput
- Replay latency
- End-to-end agent loop overhead

**Run time**: ~3-4 minutes (full mode), ~45 seconds (quick mode)

---

### 3. `quick_clone_test.rs` - Fast Comparison

**What it measures**:
- Direct comparison of owned vs Arc cloning for 20, 100, 500 messages
- Minimal benchmark for quick iteration

**Key metrics**:
- Clone time (owned)
- Clone time (Arc)
- Speedup factor

**Run time**: ~15 seconds

---

## Interpreting Results

### Criterion Output Format

```
clone_20_owned          time:   [735.58 ns 747.76 ns 763.19 ns]
                              ↑          ↑         ↑
                         Lower bound  Estimate  Upper bound
```

- **Estimate**: Best performance estimate (median)
- **Lower/Upper bounds**: 95% confidence interval
- **Outliers**: Measurements significantly different from the norm

### Performance Metrics

- **ns** (nanoseconds): 1 billionth of a second (10⁻⁹ s)
- **µs** (microseconds): 1 millionth of a second (10⁻⁶ s)
- **ms** (milliseconds): 1 thousandth of a second (10⁻³ s)

**Rule of thumb**:
- < 1 µs: Negligible overhead
- 1-10 µs: Low overhead (acceptable for most use cases)
- 10-100 µs: Medium overhead (optimize if frequent)
- > 100 µs: High overhead (likely bottleneck)

### Throughput Metrics

- **elem/s**: Elements (messages/patches) processed per second
- **MiB/s**: Megabytes per second (for state cloning)

Higher is better.

---

## Benchmark Results Location

After running benchmarks, results are saved to:

```
target/criterion/
├── clone_20_owned/
│   ├── base/
│   │   └── estimates.json
│   └── report/
│       └── index.html     ← Open in browser for interactive charts
├── clone_100_arc/
│   └── ...
└── ...
```

**View HTML reports**:
```bash
# Open all reports
open target/criterion/report/index.html

# Or for specific benchmark
open target/criterion/clone_100_owned/report/index.html
```

---

## Comparing Performance Changes

### Baseline Workflow

1. **Establish baseline** (before optimization):
   ```bash
   cargo bench --package carve-agent --bench quick_clone_test -- --save-baseline before
   ```

2. **Make your changes** (e.g., implement Arc<Message>)

3. **Compare against baseline**:
   ```bash
   cargo bench --package carve-agent --bench quick_clone_test -- --baseline before
   ```

Criterion will show percentage improvements/regressions.

---

## Performance Tuning Tips

### Reduce Benchmark Noise

1. **Close unnecessary applications** (browsers, IDEs)
2. **Disable CPU frequency scaling**:
   ```bash
   sudo cpupower frequency-set --governor performance
   ```
3. **Pin to specific CPU cores** (Linux):
   ```bash
   taskset -c 0 cargo bench --package carve-agent --bench quick_clone_test
   ```

### Quick Iteration

For rapid development, use `--quick` mode:
```bash
cargo bench --package carve-agent --bench quick_clone_test -- --quick
```

This reduces sample count from 100 to ~20, making benchmarks 3-5x faster.

---

## Expected Performance Characteristics

### Current Implementation (Owned Messages)

| Conversation Length | Expected Clone Time |
|---------------------|---------------------|
| 20 messages         | ~750 ns             |
| 100 messages        | ~5 µs               |
| 500 messages        | ~25-30 µs           |

**Scaling**: O(n) linear with message count

### Optimized Implementation (Arc Messages)

| Conversation Length | Expected Clone Time |
|---------------------|---------------------|
| 20 messages         | ~150 ns             |
| 100 messages        | ~700 ns             |
| 500 messages        | ~3.5 µs             |

**Scaling**: O(n) linear but with much smaller constant factor (~7x improvement)

---

## Common Issues

### Benchmark Won't Compile

```bash
# Ensure criterion is in dev-dependencies
cargo update criterion

# Check Cargo.toml has:
# [dev-dependencies]
# criterion.workspace = true
```

### Results Show High Variance

- Check for background processes (antivirus, indexing)
- Ensure system is not thermal throttling
- Run benchmarks multiple times and average results

### Gnuplot Not Found Warning

This is cosmetic. Criterion will use plotters backend instead.

To install gnuplot (optional):
```bash
# Ubuntu/Debian
sudo apt install gnuplot

# macOS
brew install gnuplot
```

---

## Adding New Benchmarks

1. **Create benchmark file**:
   ```bash
   touch crates/carve-agent/benches/my_bench.rs
   ```

2. **Add to Cargo.toml**:
   ```toml
   [[bench]]
   name = "my_bench"
   harness = false
   ```

3. **Write benchmark**:
   ```rust
   use criterion::{black_box, criterion_group, criterion_main, Criterion};

   fn bench_my_function(c: &mut Criterion) {
       c.bench_function("my_function", |b| {
           b.iter(|| {
               // Code to benchmark
               black_box(my_function());
           });
       });
   }

   criterion_group!(benches, bench_my_function);
   criterion_main!(benches);
   ```

4. **Run**:
   ```bash
   cargo bench --package carve-agent --bench my_bench
   ```

---

## CI/CD Integration

### GitHub Actions Example

```yaml
name: Performance Regression Tests

on: [push, pull_request]

jobs:
  benchmark:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v2

      - name: Run benchmarks
        run: cargo bench --package carve-agent -- --save-baseline ci

      - name: Upload results
        uses: actions/upload-artifact@v2
        with:
          name: benchmark-results
          path: target/criterion/
```

For regression detection, use tools like [criterion-compare-action](https://github.com/boa-dev/criterion-compare-action).

---

## Further Reading

- [Criterion.rs User Guide](https://bheisler.github.io/criterion.rs/book/)
- [Rust Performance Book](https://nnethercote.github.io/perf-book/)
- [Flamegraph Profiling](https://github.com/flamegraph-rs/flamegraph)

---

## Summary

**Quick benchmark**: `cargo bench --package carve-agent --bench quick_clone_test`

**Full analysis**: `cargo bench --package carve-agent`

**View results**: `open target/criterion/report/index.html`

For detailed performance analysis, see [`PERFORMANCE_ANALYSIS.md`](./PERFORMANCE_ANALYSIS.md).
