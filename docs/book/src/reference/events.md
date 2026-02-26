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
- `ReasoningDelta` / `ReasoningEncryptedValue`
- `StateSnapshot` / `StateDelta`
- `MessagesSnapshot`
- `ActivitySnapshot` / `ActivityDelta`
- `ToolCallResumed`
- `Error`

## Terminal Semantics

`RunFinish.termination` indicates why the run ended and should be treated as authoritative.
