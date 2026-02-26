# Run Lifecycle and Phases

The runtime uses a two-layer state machine: a **run-level** state machine tracks
coarse execution status, while a **tool-call-level** state machine tracks each
tool call independently. Suspension bridges the two layers — when all active tool
calls are suspended, the run transitions to `Waiting`; when a resume decision
arrives, the run transitions back to `Running`.

## Two-Layer State Machine

### Layer 1: Run Lifecycle (`RunStatus`)

Persisted at `state["__run"]`.

```mermaid
stateDiagram-v2
    [*] --> Running
    Running --> Waiting: all tool calls suspended
    Running --> Done: NaturalEnd / PluginRequested / Stopped / Cancelled / Error
    Waiting --> Running: resume decision received
    Waiting --> Done: Cancelled / Error
```

| Status | Meaning |
|--------|---------|
| `Running` | Run is actively executing (inference or tools) |
| `Waiting` | Run is paused waiting for external resume decisions |
| `Done` | Terminal — run finished with a `TerminationReason` |

Terminal reasons: `NaturalEnd`, `PluginRequested`, `Stopped(code)`, `Cancelled`,
`Suspended`, `Error`.

### Layer 2: Tool Call Lifecycle (`ToolCallStatus`)

Persisted per call at `state["__tool_call_states"]`.

```mermaid
stateDiagram-v2
    [*] --> New
    New --> Running
    New --> Suspended: pre-execution suspend
    Running --> Suspended: tool returned Pending
    Running --> Succeeded: tool returned Success/Warning
    Running --> Failed: tool returned Error
    Running --> Cancelled: run cancelled
    Suspended --> Resuming: resume decision received
    Suspended --> Cancelled: cancel decision or run cancelled
    Resuming --> Running: replay tool call
    Resuming --> Suspended: re-suspended after replay
    Resuming --> Succeeded
    Resuming --> Failed
    Resuming --> Cancelled
```

| Status | Meaning |
|--------|---------|
| `New` | Call observed but not yet started |
| `Running` | Call is executing |
| `Suspended` | Call paused waiting for external decision |
| `Resuming` | External decision received, replay in progress |
| `Succeeded` | Terminal — execution succeeded |
| `Failed` | Terminal — execution failed |
| `Cancelled` | Terminal — call cancelled |

### How the Layers Connect

1. During a tool round, each tool call transitions through its own lifecycle.
2. After the round commits, the run evaluates all outcomes:
   - If **all** outcomes are `Suspended` → run transitions to `Waiting` and
     terminates with `TerminationReason::Suspended`.
   - If **any** outcome is non-suspended → run stays `Running` and loops back
     to inference.
3. An inbound `ToolCallDecision` triggers:
   - Tool call: `Suspended` → `Resuming` → replay → terminal state.
   - Run: `Waiting` → `Running` (if the run was suspended).

### Durable State Paths

| Path | Content |
|------|---------|
| `__run` | `RunState` (id, status, done_reason, updated_at) |
| `__tool_call_states` | `ToolCallStatesMap` (per-call status map) |
| `__suspended_tool_calls` | `SuspendedToolCallsState` (suspended call payloads) |

## Canonical Top-Level Flow

1. `RunStart`
2. `commit(UserMessage)` + optional resume replay (`drain_resume_decisions_and_replay`)
3. Loop:
   - `RESUME_TOOL_CALL` (apply inbound decisions + replay if any)
   - `StepStart`
   - `BeforeInference`
   - `LLM_CALL`
   - `AfterInference`
   - `StepEnd`
   - Optional `TOOL_CALL` round:
     - `BeforeToolExecute`
     - Tool execution
     - `AfterToolExecute`
     - apply tool results + commit
     - `RESUME_TOOL_CALL` again (consume decisions received during tool round)
4. `RunEnd`

This loop applies to both `run_loop` and `run_loop_stream`; stream mode adds
extra decision handling windows while inference/tool execution is in-flight.

## Run Execution Flow (non-stream canonical)

```mermaid
stateDiagram-v2
    [*] --> RunStart
    RunStart --> CommitUser: CKPT UserMessage
    CommitUser --> ResumeReplay0
    ResumeReplay0 --> CommitReplay0: replayed
    ResumeReplay0 --> LoopTop: no replay
    CommitReplay0 --> LoopTop
    CommitReplay0 --> Suspended: new suspended at run-start
    LoopTop --> ResumeToolCall
    ResumeToolCall --> StepPrepare
    StepPrepare --> Terminate: run_action=Terminate
    StepPrepare --> LlmCall: run_action=Continue
    LlmCall --> Error
    LlmCall --> Cancelled
    LlmCall --> AfterInference
    AfterInference --> CommitAssistant: CKPT AssistantTurnCommitted
    CommitAssistant --> Terminate: post-inference terminate
    CommitAssistant --> NaturalEnd: no tool calls
    CommitAssistant --> ToolRound: has tool calls
    ToolRound --> Error
    ToolRound --> Cancelled
    ToolRound --> CommitTool: CKPT ToolResultsCommitted
    CommitTool --> ResumeAfterTool
    ResumeAfterTool --> Suspended: all outcomes suspended
    ResumeAfterTool --> LoopTop: has non-suspended outcomes
    Terminate --> RunEnd
    NaturalEnd --> RunEnd
    Suspended --> RunEnd
    Cancelled --> RunEnd
    Error --> RunEnd
    RunEnd --> CommitFinished: CKPT RunFinished(force=true)
    CommitFinished --> [*]
```

## Checkpoint Triggers

`StateCommitter` is optional. When configured, checkpoints are emitted at these
boundaries:

1. `UserMessage`
   - Trigger: after `RunStart` side effects are applied.
2. `ToolResultsCommitted` (run-start replay)
   - Trigger: run-start replay actually executed at least one resumed call.
3. `AssistantTurnCommitted`
   - Trigger: each successful `LLM_CALL` result is applied (`AfterInference` + assistant message + `StepEnd`).
4. `ToolResultsCommitted` (tool round)
   - Trigger: each completed tool round is applied to session state/messages.
5. `ToolResultsCommitted` (decision replay in-loop)
   - Trigger: inbound decision resolved and replay produced effects.
6. `RunFinished` (`force=true`)
   - Trigger: any terminal exit path (`NaturalEnd` / `Suspended` / `Cancelled` / `Error` / plugin-requested termination).

## Resume Gate After Tool Commit

After `CommitTool`, the loop does **not** jump directly to `LLM_CALL`. It always
passes through `RESUME_TOOL_CALL` first (`apply_decisions_and_replay`).

Why:

- consume decisions that arrived during tool execution;
- avoid stale `Suspended` evaluation;
- persist replay side effects before the next inference step.

So the transition is:

`CommitTool -> RESUME_TOOL_CALL -> StepStart/BeforeInference -> LLM_CALL`.

## RESUME_TOOL_CALL Sub-State Machine (Decision Replay)

`RESUME_TOOL_CALL` is the decision-drain + replay gate used at loop top and
immediately after tool-round commit.

```mermaid
stateDiagram-v2
    [*] --> DrainDecisionChannel
    DrainDecisionChannel --> NoDecision: channel empty / unresolved only
    DrainDecisionChannel --> ResolveDecision: matched suspended call
    ResolveDecision --> PersistResumeDecision: upsert __resume_decisions
    PersistResumeDecision --> Replay
    Replay --> ReplayCancel: action=Cancel
    Replay --> ReplayResume: action=Resume
    ReplayCancel --> ClearResolved
    ReplayResume --> ExecuteReplayedTool
    ExecuteReplayedTool --> ClearResolved: outcome != Suspended
    ExecuteReplayedTool --> SetNextSuspended: outcome = Suspended
    SetNextSuspended --> ClearResolved
    ClearResolved --> SnapshotIfStateChanged
    SnapshotIfStateChanged --> CommitReplay: CKPT ToolResultsCommitted
    CommitReplay --> [*]
    NoDecision --> [*]
```

Notes:

- `NoDecision` exits with no replay commit.
- `CommitReplay` is only triggered when at least one decision is resolved and
  replay path executes.
- The same sub-state machine is reused in:
  - run-start drain path;
  - loop-top `apply_decisions_and_replay`;
  - post-tool `apply_decisions_and_replay`.

## TOOL_CALL Sub-State Machine (Per ToolCall)

This diagram shows how plugin phases drive `ToolCallStatus` transitions
within a single tool-call round. The outcome (`ToolCallOutcome`) maps directly
to the terminal `ToolCallStatus` values described in Layer 2 above.

```mermaid
stateDiagram-v2
    [*] --> BeforeToolExecute
    BeforeToolExecute --> Blocked: plugin block / tool_blocked
    BeforeToolExecute --> Suspended: plugin suspend / tool_pending
    BeforeToolExecute --> Failed: out-of-scope / tool-missing / arg-invalid
    BeforeToolExecute --> ExecuteTool: allow
    ExecuteTool --> Succeeded: ToolResult success/warning
    ExecuteTool --> Failed: ToolResult error
    ExecuteTool --> Suspended: ToolResult pending
    Succeeded --> AfterToolExecute
    Failed --> AfterToolExecute
    Blocked --> AfterToolExecute
    Suspended --> AfterToolExecute
    AfterToolExecute --> [*]
```

Outcome-to-status mapping:

| `ToolCallOutcome` | `ToolCallStatus` |
|--------------------|------------------|
| `Succeeded` | `Succeeded` |
| `Failed` | `Failed` |
| `Suspended` | `Suspended` |

Important:

- `Blocked` is a **pre-execution gate** outcome (from `BeforeToolExecute`) and
  is returned to model as `ToolResult::error` with `ToolCallOutcome::Failed`.
- In canonical loop execution, `ExecuteTool` itself does not produce a
  cancellation-flavored per-tool outcome; execution-time cancellation is handled
  at run/tool round level (see below).

## CancellationToken Impact on TOOL_CALL

`CancellationToken` affects TOOL_CALL at **round/executor level**:

- If token is cancelled while waiting tool round completion, executor returns
  cancellation error and run terminates with `TerminationReason::Cancelled`.
- This is not represented as a per-tool blocked/failed gate result.

So there are two different "cancel" semantics:

1. **ToolCallOutcome::Failed**: plugin/tool-gate blocks and returns tool error.
2. **TerminationReason::Cancelled**: external run cancellation token aborts run.

## Parallel Tool Execution and Incoming Resume

### Non-stream (`run_loop`)

- During `tool_executor.execute(...).await`, decision channel is not drained.
- Incoming decisions stay queued.
- They are applied in the immediate post-tool `RESUME_TOOL_CALL` phase.

### Stream (`run_loop_stream`)

- Tool round uses `tokio::select!` over:
  - tool future,
  - activity channel,
  - decision channel.
- Incoming decisions are applied immediately (`apply_decision_and_replay`).
- If a suspended call is already resolved during this window, returned
  suspended result for that call is filtered to avoid duplicate pending events.

## Clarifications for Common Questions

### 1) Tool failure vs loop error

- Tool-level failures (tool not found, arg validation failure, plugin block,
  tool returned error result) are written as tool result messages and the loop
  continues to next inference round.
- Runtime-level errors (tool executor failure, patch/apply failure, checkpoint
  commit failure) terminate with `TerminationReason::Error`.

### 2) `CommitTool` granularity

- `CommitTool` is per **tool round batch** (`Vec<ToolExecutionResult>`), not per
  single tool call.
- In sequential executor, the batch may be a prefix if execution stops at first
  suspended call.

### 3) Why post-tool replay is required

After tool batch commit, loop runs `apply_decisions_and_replay` again to:

- consume decisions that arrived while tools were running;
- prevent stale suspended-state checks;
- make replay side effects durable before next `LLM_CALL`.

### 4) Transition when there are non-suspended tool results

When tool results contain at least one non-suspended outcome, the run does not
terminate. Control returns to loop top:

`CommitTool -> RESUME_TOOL_CALL -> StepStart/BeforeInference -> LLM_CALL`.

## Why This Design

- Phases isolate extension logic in plugins.
- Run termination is explicit via `TerminationReason`.
- Step-local control (tool filtering, tool suspension) is deterministic.
