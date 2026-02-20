# Debug a Run

Use this when a run stops unexpectedly or tool behavior is incorrect.

## Prerequisites

- Target `thread_id` and (if available) `run_id`.
- Access to the emitted event stream and persisted thread data.

## Steps

1. Confirm termination reason from `AgentEvent::RunFinish { termination, .. }`.

`termination` is authoritative and usually one of:

- `NaturalEnd`
- `Stopped(...)`
- `PendingInteraction`
- `Cancelled`
- `Error`

2. Inspect event timeline ordering:

- `StepStart` / `StepEnd`
- `ToolCallStart` / `ToolCallDone`
- `InferenceComplete`
- `Error`

3. Verify persisted delta in storage:

- New messages
- New patches
- Metadata version increment

4. Check plugin phase behavior if execution is phase-dependent:

- `BeforeInference`
- `BeforeToolExecute`
- `AfterToolExecute`

5. Reproduce with minimal deterministic inputs and compare event traces.

## Verify

A fix is effective when:

- The same input no longer reproduces the failure.
- Event sequence and terminal reason are stable across repeated runs.
- Persisted thread state matches expected messages and patch history.

## Common Errors

- Debugging only final text output and ignoring event stream.
- Inspecting latest thread snapshot but not patch delta/version movement.
- Mixing protocol-encoded events with canonical `AgentEvent` semantics.

## Related

- [Events](../reference/events.md)
- [Thread Model](../reference/thread-model.md)
- [Run Lifecycle and Phases](../explanation/run-lifecycle-and-phases.md)
