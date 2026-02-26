# Changelog

All notable changes to this project will be documented in this file.

## [Unreleased]

- Initial open-source baseline files added (`README`, `LICENSE*`, `SECURITY`, `CONTRIBUTING`, `CODE_OF_CONDUCT`).
- CI test workflow added for `cargo test --workspace`.
- Repository link consistency fix in docs configuration.

## [0.2.0] - 2026-02-26

### Breaking Changes

#### Module Reorganization

- Consolidated `event` and `protocol` modules into unified `io` module.
  - `AgentEvent` now at `io::event`.
  - `RunRequest` and `RuntimeInput` at `io::input`.
  - `ToolCallDecision` and `ResumeDecisionAction` at `io::decision`.
  - `RuntimeOutput` at `io::output`.
- New `transport` module with `Transcoder` trait and `Identity<T>` pass-through.
  - Removed `ProtocolInputAdapter` trait.
  - Removed `ProtocolOutputEncoder` trait.
  - Removed `ProtocolHistoryEncoder` trait.
- Removed protocol shim module and legacy combined HTTP/NATS entrypoints.
- Removed `tirea-interaction-plugin` crate; tests migrated to local fixtures.

#### Plugin Interface

- Replaced `on_phase(Phase, StepContext)` with individual per-phase methods on `AgentPlugin`:
  `run_start`, `step_start`, `before_inference`, `after_inference`, `before_tool_execute`,
  `after_tool_execute`, `step_end`, `run_end` — each with a typed context parameter.
- Removed `StepContext` legacy skip/termination fields.
- Removed `ask_frontend_tool` phase helper and legacy `ask`/`allow`/`deny` tool-gate helpers.
- `BeforeToolExecute` unified to block/allow/suspend semantics.
- Renamed `skip_inference` to `terminate_plugin_requested`.

#### Suspension and Tool-Call Lifecycle

- Replaced "pending interaction" model with per-call tool-call suspension.
  - `pending_interaction()` / `pending_frontend_invocation()` → `suspended_calls()`.
  - Single-slot coalesce replaced by per-call `SuspendedCall` with `call_id`, `tool_name`,
    `arguments`, `suspension`, `pending`, `resume_mode`.
  - Resume via `ToolCallDecision` on typed decision channel.
- Decision channel switched to `ToolCallDecision` type.
- Tool execution switched to outcome-based contract (`ToolCallOutcome`).
- Tool gating migrated to proceed/suspend/cancel model.

#### Event Stream

- Removed `AgentEvent` variants: `InteractionRequested`, `InteractionResolved`, `Pending`.
- Added `AgentEvent` variants: `ReasoningDelta`, `ReasoningEncryptedValue`, `ToolCallResumed`.
- `InferenceComplete` now includes `duration_ms` field.

#### Stop Conditions

- Moved stop-condition evaluation from core loop to `StopPolicyPlugin`.
- `StopPolicy` trait with `evaluate()` method.
- Declarative `StopConditionSpec` config (MaxRounds, Timeout, TokenBudget,
  ConsecutiveErrors, StopOnTool, ContentMatch, LoopDetection).

#### `RunRequest` Fields

- Added `initial_decisions: Vec<ToolCallDecision>` (required, no default).
- Added `parent_run_id: Option<String>`.

#### Type Renames (deprecated aliases provided)

- `RunLifecycleStatus` → `RunStatus`
- `RunLifecycleState` → `RunState`
- `RunLifecycleAction` → `RunAction`
- `ToolCallLifecycleAction` → `ToolCallAction`
- `ToolCallLifecycleState` → `ToolCallState`
- `ToolCallLifecycleStatesState` → `ToolCallStatesMap`
- `ToolSuspension` → `SuspendTicket`

#### Method Renames (deprecated forwarding methods provided)

- `StateManager::apply()` → `StateManager::commit()`
- `StateManager::apply_batch()` → `StateManager::commit_batch()`

### New Features

- **Two-layer state machine**: Run-level (`RunStatus`: Running/Waiting/Done) and tool-call-level
  (`ToolCallStatus`: New/Running/Suspended/Resuming/Succeeded/Failed/Cancelled) with suspension
  bridging the two layers.
- **Durable run lifecycle**: `RunState` persisted at `state["__run"]` with id, status, done_reason,
  and updated_at fields.
- **Tool-call lifecycle enforcement**: `ToolCallStatus` transitions validated at runtime; illegal
  transitions rejected.
- **Tool-call state persistence**: Per-call `ToolCallState` at `state["__tool_call_states"]` and
  `SuspendedToolCallsState` at `state["__suspended_tool_calls"]`.
- **MCP progress notifications**: Progress reporting support in tool calls over HTTP transport.
- **Transport binding contracts**: Typed upstream/downstream binding with `RuntimeEndpoint`,
  `EndpointPair`, `TranscoderEndpoint`, and `wire_http_sse_relay` helper.
- **Approval policies**: Carry, strict, and auto-cancel pending approval policies for tool
  execution.
- **Tool execution modes**: Configurable parallel tool execution with explicit mode selection.
- **Plugin action primitives**: `RunAction`, `ToolCallAction`, `StateEffect` available in phase
  contexts for fine-grained run/tool control.
- **Decision replay in stream mode**: Incoming decisions consumed and replayed during in-flight
  tool execution via `tokio::select!`.
- **AI SDK incremental messages**: Frontend sends incremental messages in AI SDK transport.

### Internal Changes

- Unified stream and non-stream termination finalization paths.
- Collapsed duplicate decision drain/replay paths into shared pipeline.
- Centralized tool-call lifecycle transition logic.
- Unified parallel tool executors by execution mode.
- Moved scope policy to `ToolPolicyPlugin`.
- Extracted shared HTTP run preparation and SSE relay.
- Decoupled genai from tirea-contract.
- Decoupled cancellation from transport layer.
- Standardized test infrastructure and helper naming.
- CI: added Rust lint and format workflow.
- Docs: enabled mdbook-mermaid and GitHub Pages deploy.

## [0.1.1] - 2026-02-24

- Workspace version baseline before public preparation updates.
