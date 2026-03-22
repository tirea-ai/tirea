# Use Reminders

Use this when reminders should be injected into inference context.

Reminders use `ContextMessage` with `consume_after_emit` to control lifecycle. The built-in `PromptSegmentsPlugin` handles emission automatically — no plugin registration required.

## Prerequisites

- `tirea` or `tirea-agentos` with prompt segments enabled.

## Steps

1. Add an ephemeral reminder (consumed after the next inference call).

```rust,ignore
use tirea::prelude::*;
use tirea_contract::runtime::inference::ContextMessage;

let action = upsert_context_message_action(
    ContextMessage::session("reminder:call-alice", "Reminder: Call Alice at 3pm")
        .with_consume_after_emit(true),
);
// dispatch as a state action
```

2. For persistent reminders that survive across inference calls:

```rust,ignore
let action = upsert_context_message_action(
    ContextMessage::session("reminder:verify-id", "Reminder: Always verify user identity"),
);
// persists until explicitly removed
```

3. Remove a specific reminder.

```rust,ignore
let action = remove_context_message_action("reminder:call-alice");
```

4. Remove all reminders by key prefix.

```rust,ignore
let action = remove_context_messages_by_prefix_action("reminder:");
```

## How It Works

Each reminder is stored as a `ContextMessage` in the `PromptSegmentState` under the `__prompt_segments` state path. Messages are keyed for deduplication and targeted removal.

- **Ephemeral**: `with_consume_after_emit(true)` — removed after it is sent to the LLM once.
- **Persistent**: default — remains until explicitly removed.
- **Target**: session context (`ContextMessageTarget::Session`).

## Common Errors

- Dispatching a reminder action but never running an inference call — the segment sits in state with no effect.
- Expecting ephemeral reminders to appear in a second inference call — they are consumed after first emission.

## Key Files

- `crates/tirea-agentos/src/runtime/prompt_segments/state.rs`
- `crates/tirea-agentos/src/runtime/prompt_segments/plugin.rs`

## Related

- [Add a Plugin](./add-a-plugin.md)
- [Run Lifecycle and Phases](../explanation/run-lifecycle-and-phases.md)
