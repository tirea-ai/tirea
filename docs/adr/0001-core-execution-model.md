# ADR-0001: Core Execution Model — Hook + Queue Hybrid

- **Status**: Accepted
- **Date**: 2026-03-21

## Context

We evaluated two existing implementations for the agent framework core:

- **awaken** (this repo): Pure queue-based model. Plugins schedule `ScheduledAction` with JSON payloads into a queue. `PhaseRuntime` consumes the queue per phase in a convergence loop. Plugins cannot self-trigger — actions must be scheduled externally.
- **tirea/uncarve** (`~/uncarve`): Pure hook-return model. `AgentBehavior` plugins implement typed async phase hooks that return `ActionSet<T>` (compile-time typed enum per phase). The loop runner directly matches and applies returned actions. No queue, no handler registry.

Neither model alone is sufficient:

| Limitation | Pure Queue (awaken) | Pure Hook (tirea) |
|---|---|---|
| Plugin autonomy | Plugins cannot self-trigger on phase change | Plugins respond to every phase |
| Extensibility | Plugin-defined action types, no core modification | New action variants require modifying core enum |
| Cross-plugin collaboration | Natural via queue | Only through shared state |
| Multi-round convergence | Native (loop until queue empty) | Requires runtime special-casing |
| Type safety | Weak (JSON serialization boundary) | Strong (compile-time ActionSet\<T\>) |

## Decision

### D1: Hook + Queue hybrid execution model

Each phase executes in three stages:

```
Phase(P):
  GATHER  — call registered phase hooks in registration order
            each hook receives a fresh snapshot and returns StateCommand { patch, actions, effects }
            patch: committed immediately; next hook snapshots from the updated store
            actions: enqueued for EXECUTE stage
            effects: dispatched immediately after commit

  EXECUTE — convergence loop (max_rounds guard)
            dequeue actions matching phase P
            handler processes action → returns StateCommand
            patch: committed; new actions re-enter queue → next round
            effects: dispatched immediately after commit
            loop until queue empty or max_rounds exceeded
```

Effects are dispatched inline on each `submit_command` call rather than collected into a separate DISPATCH stage. This keeps the model simple — each StateCommand is fully resolved (state committed, effects fired) before the next hook or handler runs.

The upper-layer agent loop is responsible for scheduling initial actions into the queue (via `submit_command`) before calling `run_phase`. The state framework does not dictate which plugins are active — it only executes registered hooks and consumes queued actions.

**Why**: Hooks give plugins autonomy (self-trigger on phase). Queue gives extensibility (plugin-defined action types) and convergence (cascading actions resolve naturally). The combination solves both pure models' limitations without requiring core enum changes for new plugin capabilities.

### D2: Effect system independent from Action system

- **Action** = internal state mutation, processed during EXECUTE with convergence loop
- **Effect** = external side effect, dispatched immediately when its containing StateCommand is committed

Effects are dispatched inline (not deferred) because each StateCommand represents a complete unit of work. The effect handler sees the snapshot produced by the commit that included the effect, which is the natural point-in-time for the side effect.

**Why the separation matters**: Actions and effects have different semantics even though both are dispatched eagerly. Actions enqueue into the convergence loop and may cascade; effects are terminal — they do not produce new actions or effects. Keeping them as distinct types prevents accidental feedback loops through the effect path.

### D3: Phase-scoped action consumption

`ScheduledAction` retains a `phase` field. During EXECUTE, only actions matching the current phase are dequeued; actions targeting other phases remain queued for their respective `run_phase` call. The upper-layer agent loop controls phase sequencing.

Cross-phase communication uses state slots when the intent is reactive (plugin A writes state, plugin B's hook reads it). Cross-phase action scheduling is available but should be used sparingly — state slots are preferred because the hook can observe the full snapshot at decision time.

**Why retain the phase field**: Removing it would require the caller to batch actions per phase before submitting, adding coordination complexity to the agent loop. Keeping it lets plugins and the loop freely submit actions that self-declare their execution phase, with the runtime filtering naturally.

### D4: Action payload serialization via `serde_json::Value`

All action and effect payloads are serialized through `serde_json::Value` at the dispatch boundary.

**Why**: Agent framework action frequency is low (single-digit to tens per phase), making JSON overhead negligible. The benefit is free auditability — action logs are human-readable and persistable without additional infrastructure. Binary or `Box<dyn Any>` alternatives were rejected: the former requires schema management, the latter loses auditability and cross-process capability.

### D5: Synchronous state engine, asynchronous hooks and handlers

```
Synchronous (no async, no tokio locks):
  StateStore.commit()
  MutationBatch operations
  SlotMap / Snapshot reads

Asynchronous (async_trait):
  Plugin phase hooks         — may read external state
  Action handlers            — may call APIs
  Effect handlers            — may write to storage
  PhaseRuntime.run_phase()   — orchestrates async hooks/handlers
```

**Why**: State engine operations are pure in-memory mutations (sub-microsecond). Making them async would force `tokio::sync::RwLock` adoption, adding complexity and potential deadlock surface with no performance benefit. The async boundary is placed where I/O actually occurs: hooks (may query external services), handlers (may invoke tools/LLMs), and effect handlers (may persist or stream).

## Consequences

### Retained from awaken

- `StateSlot` + `SlotMap` + `Snapshot` — type-safe heterogeneous state container
- `StateStore` + `MutationBatch` — transactional commit with optimistic locking
- `StatePlugin` — plugin lifecycle (register slots, install/uninstall hooks)
- `StateCommand` — unified return type combining patch + actions + effects
- `ScheduledAction` + handler registry — action dispatch infrastructure
- `TypedEffect` + handler registry — effect dispatch infrastructure
- `CommitHook` — state change observability

### Already implemented

- `PhaseRuntime.run_phase()` — GATHER (hook invocation) before EXECUTE convergence loop
- `PhaseHook` trait + `PluginRegistrar::register_phase_hook()` — hook registration per phase
- `PhaseRuntime.install_plugin()` / `uninstall_plugin()` — hook lifecycle tied to plugin
- Effect immediate dispatch in `submit_command_inner()`

### To be redesigned

- Plugin trait / handler traits — make async (D5)

### To be added (not in current awaken)

- State scope (Thread / Run / ToolCall) on `SlotOptions`
- Scope-aware lifecycle management in `StateStore`
- Phase enum expansion: 6 → 8 (add `StepStart` / `StepEnd`) when streaming protocol support is needed

### To be built in separate crates

- Agent loop (inference → tool execution → state → checkpoint)
- Tool trait + ToolRegistry
- Message / Thread / Conversation model
- LLM provider abstraction
- Storage SPI
- Event streaming
- Multi-agent orchestration
