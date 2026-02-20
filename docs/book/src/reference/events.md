# Events

`AgentEvent` is the canonical run event stream.

## Lifecycle

- `RunStart`
- `StepStart`
- `InferenceComplete`
- `ToolCallStart` / `ToolCallDelta` / `ToolCallReady` / `ToolCallDone`
- `StepEnd`
- `RunFinish`

## State and UI Events

- `TextDelta`
- `StateSnapshot` / `StateDelta`
- `MessagesSnapshot`
- `ActivitySnapshot` / `ActivityDelta`
- `InteractionRequested` / `InteractionResolved` / `Pending`
- `Error`

## Terminal Semantics

`RunFinish.termination` indicates why the run ended and should be treated as authoritative.
