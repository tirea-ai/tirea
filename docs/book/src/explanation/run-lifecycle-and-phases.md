# Run Lifecycle and Phases

The runtime executes one run as a phase-driven loop.

## Top-Level Flow

1. `RunStart`
2. Repeated steps:
   - `StepStart`
   - `BeforeInference`
   - LLM inference
   - `AfterInference`
   - `BeforeToolExecute`
   - Tool execution
   - `AfterToolExecute`
   - `StepEnd`
3. `RunEnd`

## Why This Design

- Phases isolate extension logic in plugins.
- Run termination is explicit via `TerminationReason`.
- Step-local control (tool filtering, pending interactions) is deterministic.

## Checkpoint Points (Streaming Path)

`run_loop_stream` persists incremental checkpoints at stable boundaries:

1. `RunStart` side effects committed as `UserMessage`.
2. RunStart replay (if any) committed as `ToolResultsCommitted`.
3. Assistant turn committed as `AssistantTurnCommitted`.
4. Tool round committed as `ToolResultsCommitted`.
5. Run termination committed as forced `RunFinished`.

This ordering guarantees that inbound side effects and replayed tool recovery are
durable before the loop advances to the next step.
