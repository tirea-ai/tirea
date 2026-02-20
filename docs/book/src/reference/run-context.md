# Run Context

`RunContext` is the mutable run-scoped workspace derived from a persisted `Thread`.

## Responsibilities

- Track run-time messages and patches
- Expose immutable snapshots of current state (`snapshot`, `snapshot_of`, `snapshot_at`)
- Emit incremental run delta via `take_delta()`
- Carry run identity and version cursor

## Common Calls

- `RunContext::from_thread(thread, run_config)`
- `messages()` / `add_message(...)`
- `add_thread_patch(...)`
- `take_delta()`
- `has_delta()`
- `pending_interaction()` / `pending_frontend_invocation()`
