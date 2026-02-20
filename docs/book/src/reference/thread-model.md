# Thread Model

`Thread` is the persisted conversation and state history entity.

## Fields

- `id`: thread identifier
- `resource_id`: optional ownership key for listing/filtering
- `parent_thread_id`: lineage for delegated/sub-agent runs
- `messages`: ordered message history
- `state`: base snapshot value
- `patches`: tracked patch history since base snapshot
- `metadata.version`: persisted version cursor

## Core Methods

- `Thread::new(id)`
- `Thread::with_initial_state(id, state)`
- `with_message` / `with_messages`
- `with_patch` / `with_patches`
- `rebuild_state()`
- `replay_to(index)`
- `snapshot()`

## Persistence Coupling

Storage append operations consume `ThreadChangeSet` plus `VersionPrecondition`.
