# ADR-0019: Mailbox Architecture — Unified Persistent Run Queue

- **Status**: Accepted
- **Date**: 2026-03-28
- **Depends on**: ADR-0012, ADR-0018
- **Supersedes**: ADR-0017 (partial — execution ownership now fully defined via mailbox)

## Context

Three components share overlapping responsibility for run scheduling:

- **RunDispatcher** — memory-only queue; crash loses all queued runs.
- **MailboxStore** — persistent push/pop, but no consumer drives it.
- **ActiveRunRegistry** — single-process; no distributed coordination possible.

No single component covers persistence, consumption, and distributed claim together.

## Decision

### D1: Single Mailbox service backed by MailboxStore trait

Replace `RunDispatcher`, the old `MailboxStore`, and `ActiveRunRegistry`'s scheduling role with a unified `Mailbox` service in `awaken-server`. `Mailbox` holds `Arc<AgentRuntime>` + `Arc<dyn MailboxStore>` and is the sole entry point for all run execution across protocols (HTTP, A2A, AG-UI, AI-SDK).

### D2: Thread-keyed routing

`mailbox_id = thread_id`. Every run request is routed by thread. Agent-targeted messages auto-generate a `thread_id` at the API level — the queue never sees `agent_id` as an address.

### D3: Write-ahead-log semantics

Every run request persists via `MailboxStore::enqueue()` before dispatch. Crash recovery replays `Queued` entries on startup via `recover()`.

### D4: Six-state lifecycle

```
Queued → Claimed → Accepted | Cancelled | Superseded | DeadLetter
```

Three terminal states (`Accepted`, `Cancelled`, `DeadLetter`). `Superseded` is terminal, triggered by thread interrupt (generation bump). `Claimed → Queued` retry via `nack()` increments `attempt_count`.

### D5: Lease-based distributed claim

Multiple processes compete for jobs through atomic `claim()` on the shared store. Each claim sets `claim_token` + `lease_until`. A renewal heartbeat extends the lease; sweep reclaims expired leases from crashed consumers. No inter-process communication needed — the store is the coordination layer.

### D6: Event-driven dispatch with sweep safety net

Normal path: `enqueue` triggers immediate dispatch if the thread's `MailboxWorker` is idle. Periodic sweep reclaims orphaned leases as a fallback. Per-thread `MailboxWorker` serializes execution (at most one active run per thread).

### D7: Crate placement

- `awaken-contract` — `MailboxJob`, `MailboxJobStatus`, `MailboxJobOrigin`, `MailboxStore` trait.
- `awaken-stores` — `InMemoryMailboxStore`, `FileMailboxStore`, `PostgresMailboxStore`.
- `awaken-server` — `Mailbox` service, `MailboxConfig`, handler integration.

## Consequences

- Crash recovery via lease expiry + startup `recover()` scan.
- Multi-process deployment without inter-process communication; the store is the coordination layer.
- Single submission path for all protocols — `Mailbox::submit()` (streaming) and `Mailbox::submit_background()` (fire-and-forget).
- `RunDispatcher`, old `MailboxStore` trait, and `MailboxEntry` are deleted.
- `AppState` replaces `dispatcher` + `mailbox_store` fields with a single `mailbox: Arc<Mailbox>`.
- ADR-0012's `ThreadRunStore` remains unchanged for checkpoint persistence; `Mailbox` orchestrates around it.
