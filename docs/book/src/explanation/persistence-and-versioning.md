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

## Why It Matters

- Prevents silent lost updates under concurrent writers.
- Keeps full history for replay and audits.
- Enables different storage backends with consistent semantics.
