# Performance Analysis: Clone Overhead in Carve Agent Framework

**Date**: 2026-02-13
**Analysis Goal**: Measure clone overhead and evaluate zero-copy optimization opportunities

## Executive Summary

**Finding**: Current implementation has **significant clone overhead** that grows linearly with conversation length.
**Recommendation**: Implement `Arc<Message>` optimization for **5-7x performance improvement** in long conversations.

---

## Benchmark Results

### 1. Message Cloning Performance

#### Owned Messages (Current Implementation)

| Conversation Length | Clone Time | Throughput |
|---------------------|------------|------------|
| 20 messages         | 748 ns     | N/A        |
| 100 messages        | 5.03 µs    | N/A        |
| 500 messages        | 26.4 µs    | N/A        |

**Scaling**: ~52 ns per message (linear growth)

#### Arc<Message> (Proposed Optimization)

| Conversation Length | Clone Time | Speedup vs Owned |
|---------------------|------------|------------------|
| 20 messages         | 149 ns     | **5.0x faster**  |
| 100 messages        | 728 ns     | **6.9x faster**  |
| 500 messages        | 3.63 µs    | **7.3x faster**  |

**Scaling**: ~7.3 ns per message (linear, but much smaller constant)

**Key Insight**: Arc cloning is essentially constant-time (atomic increment + pointer copy), while owned cloning requires deep copy of all strings and metadata.

---

### 2. State Cloning Performance

| State Size | Bytes | Clone Time | Throughput   |
|------------|-------|------------|--------------|
| Small      | 100   | 192 ns     | 496 MiB/s    |
| Medium     | 356   | 603 ns     | 563 MiB/s    |
| Large      | 7981  | 13.7 µs    | 553 MiB/s    |

**Observation**: State cloning is relatively fast (500+ MiB/s) even for large JSON documents. Not a bottleneck for typical agent states.

---

### 3. Session Operations Performance

#### rebuild_state (apply all patches to base state)

| Patch Count | Time      | Throughput        |
|-------------|-----------|-------------------|
| 0           | 593 ns    | (baseline clone)  |
| 10          | 7.59 µs   | 1.32 M patches/s  |
| 50          | 36.1 µs   | 1.38 M patches/s  |
| 100         | 74.7 µs   | 1.34 M patches/s  |
| 500         | 514 µs    | 0.97 M patches/s  |

**Observation**: Patch application is efficient (~700 ns per patch on average).

#### replay_to (time-travel debugging)

| Target Index | Time     |
|--------------|----------|
| 0            | 1.32 µs  |
| 25           | 19.3 µs  |
| 50           | 38.6 µs  |
| 75           | 56.8 µs  |
| 99           | 75.4 µs  |

**Scaling**: ~760 ns per patch (linear, as expected for sequential application).

#### Full Session Clone (messages + state + patches)

| Scenario      | Messages | Patches | Clone Time | Throughput     |
|---------------|----------|---------|------------|----------------|
| Short         | 20       | 10      | 2.96 µs    | 10.1 M elem/s  |
| Medium        | 100      | 50      | 11.2 µs    | 13.4 M elem/s  |
| Long          | 500      | 200     | 46.8 µs    | 14.9 M elem/s  |

**Critical Observation**: As conversations grow, the message clone overhead dominates.

---

## Performance Impact Analysis

### Current Bottlenecks

#### 1. **LLM Request Building** (Highest Impact)

Every LLM call requires cloning the entire message history:

```rust
// Current implementation (loop.rs:687)
messages.extend(step.session.messages.clone());  // ⚠️ Clones all messages
```

**Impact at 100 messages**: 5 µs clone overhead per LLM call
**Impact at 500 messages**: 26 µs clone overhead per LLM call

For a 10-turn conversation with 50 initial messages:
- **Current**: 10 × 5 µs = 50 µs wasted on clones
- **With Arc**: 10 × 0.7 µs = 7 µs (7x improvement)

#### 2. **Concurrent Tool Execution** (Medium Impact)

Each parallel tool spawns a task with cloned context:

```rust
// loop.rs:1229-1235
let state = state.clone();          // Clone state
let scratchpad = scratchpad.clone(); // Clone scratchpad
```

**Impact**: For 3 concurrent tools, we clone state 3 times per step.

#### 3. **Session Storage** (Low Impact)

Cloning sessions for persistence is infrequent and acceptable overhead.

---

## Optimization Recommendations

### Priority 1: Arc<Message> (HIGH IMPACT)

**Change**:
```rust
// From:
pub struct Thread {
    pub messages: Vec<Message>,
    // ...
}

// To:
pub struct Thread {
    pub messages: Vec<Arc<Message>>,
    // ...
}
```

**Benefits**:
- 5-7x faster message cloning
- Reduced memory allocations
- Thread-safe sharing (bonus for future concurrency)

**Cost**:
- Arc atomic increment/decrement (negligible)
- Slightly larger message size (+8 bytes per Arc)

**ROI**: Extremely high for conversations >50 messages

**Estimated effort**: 2-4 hours (update types, adjust serialization, test)

---

### Priority 2: Scratchpad Arc Sharing (MEDIUM IMPACT)

**Change**:
```rust
// From:
let scratchpad = scratchpad.clone(); // in tool execution

// To:
let scratchpad = Arc::new(scratchpad);
// or use Arc<RwLock<HashMap>> for shared mutable access
```

**Benefits**:
- Avoid HashMap clone for each concurrent tool
- Better cache locality

**Cost**:
- Slightly more complex API (Arc wrapper)

**ROI**: Medium for high-concurrency scenarios (>5 parallel tools)

---

### Priority 3: State View Pattern (LOW PRIORITY)

**Change**: Instead of cloning full state, pass lightweight views:

```rust
pub struct StateView<'a> {
    base: &'a Value,
    patches: &'a [TrackedPatch],
}

impl StateView<'_> {
    fn current(&self) -> CarveResult<Value> {
        apply_patches(self.base, self.patches.iter().map(|p| p.patch()))
    }
}
```

**Benefits**:
- Avoid state clone until actually needed (lazy evaluation)

**Cost**:
- Lifetime annotations
- Higher implementation complexity

**ROI**: Low (state cloning is already fast at 500+ MiB/s)

---

## When to Optimize

### **Optimize Now** if:
- ✅ Conversations regularly exceed 100 messages
- ✅ High-frequency agent loops (>10 turns per session)
- ✅ Concurrent tool execution is common

### **Keep Current Design** if:
- ❌ Most conversations are <20 messages
- ❌ Agent loops are infrequent (<5 turns)
- ❌ Simplicity is valued over 5-10 µs savings

---

## Performance Targets After Optimization

With `Arc<Message>` implementation:

| Conversation Length | Current Clone | Optimized Clone | Improvement |
|---------------------|---------------|-----------------|-------------|
| 100 messages        | 5.0 µs        | 0.73 µs         | 6.9x        |
| 500 messages        | 26.4 µs       | 3.6 µs          | 7.3x        |
| 1000 messages       | ~53 µs        | ~7 µs           | 7.6x        |

**Total speedup for 10-turn loop with 100-message history**: ~50 µs saved

---

## Conclusion

**Key Findings**:
1. Message cloning is the **primary bottleneck** (5-26 µs per clone)
2. Arc<Message> provides **5-7x speedup** with minimal downsides
3. State cloning is already efficient (not a bottleneck)
4. Patch application is fast (~700 ns per patch)

**Recommended Action**:
Implement `Arc<Message>` optimization for conversations expected to exceed 50 messages. The ROI is high, implementation is straightforward, and it aligns with the immutable architecture.

**Next Steps**:
1. Create feature branch for Arc<Message> migration
2. Update Thread/Session types
3. Adjust serialization (transparent with serde)
4. Run regression tests
5. Re-benchmark to verify 5-7x improvement
