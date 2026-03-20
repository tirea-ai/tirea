# ADR-0002: State Engine Design

- **Status**: Accepted
- **Date**: 2026-03-21

## Context

The state engine is the foundation layer for the agent framework. All plugin state, runtime bookkeeping, and cross-phase communication flow through it. Four interrelated design choices define its character. These choices were made during the initial port (commits `97a5417`..`051fcd2`) and are retained by ADR-0001.

Alternatives considered include tirea/uncarve's JSON-patch-based `Doc` model, generic `HashMap<String, Box<dyn Any>>` containers, and pessimistic locking via `RwLock` guards.

## Decisions

### D1: Typed heterogeneous state via `StateSlot` trait + `TypedMap`

Each state slot is a Rust type implementing the `StateSlot` trait:

```rust
trait StateSlot: 'static {
    const KEY: &'static str;
    type Value: Clone + Default + Serialize + DeserializeOwned + Send + Sync;
    type Update: Send;
    fn apply(value: &mut Self::Value, update: Self::Update);
}
```

Slot values are stored in a `TypedMap` (type-erased `HashMap<TypeId, Box<dyn Any>>`) and accessed via generic methods that recover the concrete type at zero runtime cost.

**Alternatives rejected**:

- **JSON document** (tirea's `Doc` model): All state lives as `serde_json::Value`, mutations are JSON patches. Gains: uniform serialization, path-level conflict detection, CRDT compatibility. Losses: no compile-time type checking on reads/writes, deserialization on every access, cannot express complex reduction logic (only CRUD operations). For an agent framework where plugins define strongly-typed domain state (permission policies, reminder lists, tool registries), compile-time safety prevents an entire class of silent-corruption bugs that JSON-patch models can only catch at runtime.

- **`HashMap<String, Box<dyn Any>>`**: Gains: simpler than TypedMap. Losses: requires manual `downcast_ref` at every call site, no centralized type registry, no compile-time proof that a slot exists. TypedMap's phantom-type approach provides the same ergonomics as a strongly-typed struct while remaining open for extension.

### D2: Optimistic locking via revision counter

`StateStore` maintains a monotonically increasing `revision: u64`. Each `MutationBatch` can optionally carry a `base_revision`. On `commit()`, if `base_revision` does not match the store's current revision, the commit is rejected with `RevisionConflict`.

**Why optimistic over pessimistic**: The state engine is synchronous (ADR-0001 D5). Within a single phase, mutations are sequential — there is no concurrent writer contention. The revision check exists to catch logic errors (stale snapshot used across phase boundaries) and to support future concurrent scenarios (parallel tool execution writing to disjoint scopes), not to serialize concurrent access. A pessimistic `RwLock`-guard model would force callers to hold locks across mutation construction, which is unnecessary overhead for the sequential case and a deadlock risk when mutations are nested.

**Why not lock-free**: Lock-free state (e.g., atomic compare-and-swap on `Arc<Snapshot>`) adds implementation complexity for a scenario that doesn't exist yet. The revision counter is trivially upgradeable to CAS semantics if needed.

### D3: Plugin-owned slot lifecycle

Slots are registered during plugin installation via `PluginRegistrar`. The plugin registry tracks which plugin owns which slots. On uninstall, owned slots are cleared (unless `retain_on_uninstall` is set). Unknown slots are rejected at commit time.

**Why not global/static declaration**: Static slot declaration (e.g., `inventory` crate or a central enum) couples all plugins at compile time. Plugin-owned registration allows:
- Dynamic plugin installation/uninstallation at runtime
- Slot validation: only registered slots can be mutated (catches typos and stale references)
- Clean teardown: uninstalling a plugin removes its state, preventing orphaned data
- Independent compilation: plugins don't need to know about each other's slot types

### D4: Immutable snapshots via `Arc<SlotMap>`

`Snapshot` wraps `Arc<SlotMap>` with a `revision` tag. Reading state always goes through a snapshot — there is no mutable reference to the live store available to plugins or handlers.

**Why not read-lock on mutable state**: Snapshots decouple readers from writers. A hook or handler can hold a snapshot indefinitely without blocking commits. This is critical for the GATHER → EXECUTE pipeline (ADR-0001 D1): hooks read a snapshot while the next hook's `patch` commits may advance the store. With read-locks, hooks would either block each other or require careful lock ordering.

The cost is a `SlotMap::clone()` on each `commit()`. In the hybrid model (ADR-0001 D1), commits occur per-hook in GATHER and per-handler in EXECUTE — more frequent than once per phase, but still low absolute count (tens per phase at most). Since `SlotMap` values are `Clone`, this cost is negligible.

## Consequences

- Plugin authors get compile-time safety: wrong slot type = compilation error, not runtime panic.
- Serialization/persistence is opt-in per slot (`SlotOptions::persistent`), not all-or-nothing.
- The `apply(value, update)` reducer pattern allows arbitrarily complex state transitions while keeping the store generic.
- Revision-based conflict detection is simple to reason about but limited to whole-store granularity. If per-slot conflict detection is needed later, the revision model can be extended with per-slot version vectors without breaking the API.
- Snapshot cloning cost scales with number of registered slots, not with state size (TypedMap entries are `Box<dyn Any>`, cloned via `SlotMap::clone()` which calls `Clone` on each value).
