# Persistence and Versioning

Thread persistence uses append-style changesets and optimistic concurrency.

## Model

- Persisted object: `Thread`
- Incremental write unit: `ThreadChangeSet`
- Concurrency guard: `VersionPrecondition::Exact(version)`

## Write Path

1. Load thread + current version.
2. Build/apply run delta (`messages`, `patches`, optional state snapshot).
3. Append with exact expected version.
4. Store returns committed next version.

## Checkpoint Mechanism

The runtime persists state through incremental checkpoints.

- Delta source: `RunContext::take_delta()`
- Delta payload: `ThreadChangeSet { run_id, parent_run_id, run_meta, reason, messages, patches, state_actions, snapshot }`
- Concurrency: append with `VersionPrecondition::Exact(version)`
- Version update: committed version is written back to `RunContext`

`snapshot` is only used when replacing base state (for example, frontend-provided
state replacement on inbound run preparation). Regular loop checkpoints are
append-only (`messages` + `patches`).

## Checkpoint Timing

### A) Inbound checkpoint (AgentOs prepare)

Before loop execution starts:

- Trigger: incoming user messages and/or inbound state replacement exist
- Reason: `UserMessage`
- Content:
  - deduplicated inbound messages
  - optional full `snapshot` when request state replaces thread state

### B) Runtime checkpoints (loop execution path)

During `run_loop` / `run_loop_stream` execution:

1. After `RunStart` phase side effects are applied:
   - Reason: `UserMessage`
   - Purpose: persist immediate inbound side effects before any replay
2. If RunStart outbox replay executes:
   - Reason: `ToolResultsCommitted`
   - Purpose: persist replayed tool outputs/patches
3. After assistant turn is finalized (`AfterInference` + assistant message + `StepEnd`):
   - Reason: `AssistantTurnCommitted`
4. After tool results are applied (including suspension state updates):
   - Reason: `ToolResultsCommitted`
5. On termination:
   - Reason: `RunFinished`
   - Forced commit (even if no new delta) to mark end-of-run boundary

## Failure Semantics

- Non-final checkpoint failure is treated as run failure:
  - emits state error
  - run terminates with error
- Final `RunFinished` checkpoint failure:
  - emits error
  - terminal run-finish event may be suppressed, because final durability was not confirmed

`AgentOs::run_stream` uses `run_loop_stream`, so production persistence follows
the same checkpoint schedule shown above.

## State Scope Lifecycle

Each `StateSpec` declares a `StateScope` that controls its cleanup lifecycle:

| Scope | Lifetime | Cleanup |
|-------|----------|---------|
| `Thread` | Persists across runs | Never cleaned automatically |
| `Run` | Per-run | Deleted by `prepare_run` before each new run |
| `ToolCall` | Per-call | Scoped under `__tool_call_scope.<call_id>`, cleaned after call completes |

### Run-scoped cleanup

At run preparation (`prepare_run`), the framework:

1. Queries `StateScopeRegistry::run_scoped_paths()` for all `Run`-scoped state paths
2. Emits `Op::delete` patches for any paths present in the current thread state
3. Applies deletions to in-memory state before the lifecycle `Running` patch

This guarantees `Run`-scoped state (e.g., `__run`, `__kernel.stop_policy_runtime`)
starts from defaults on every new run, preventing cross-run leakage.

### Choosing a scope when authoring state

| State shape | Recommended scope | Why |
|---|---|---|
| User-visible business state (threads, notes, trips, reports) | `Thread` | Must survive across runs and reloads |
| Execution bookkeeping (`__run`, stop-policy counters, per-run temp state) | `Run` | Useful only while one run is active and must not leak into the next run |
| Pending approval / per-call scratch state | `ToolCall` | Bound to a single tool invocation and cleaned when that call resolves |

In practice:

- prefer `Thread` for state a user would expect to see after a page reload;
- prefer `Run` for coordination state owned by plugins or the runtime;
- prefer `ToolCall` when the data only makes sense while a specific suspended call exists.

## Why It Matters

- Prevents silent lost updates under concurrent writers.
- Keeps full history for replay and audits.
- Enables different storage backends with consistent semantics.
