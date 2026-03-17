# Migration Notes

## 0.2 → 0.3

### Behavior Trait Rename and Signature Overhaul

`AgentPlugin` is renamed to `AgentBehavior`. Phase hooks now receive `&ReadOnlyContext<'_>`
(immutable) instead of `&mut *Context` (mutable), and return `ActionSet<PhaseAction>` instead
of mutating context directly.

```rust,ignore
// Before (0.2)
use tirea::prelude::*; // imports AgentPlugin

#[async_trait]
impl AgentPlugin for MyPlugin {
    fn id(&self) -> &str { "my_plugin" }
    async fn before_inference(&self, ctx: &mut BeforeInferenceContext<'_, '_>) {
        ctx.add_system_context("Time: now".into());
        ctx.exclude_tool("dangerous_tool");
    }
}

// After (0.3)
use tirea::prelude::*; // imports AgentBehavior, ActionSet, BeforeInferenceAction

#[async_trait]
impl AgentBehavior for MyPlugin {
    fn id(&self) -> &str { "my_plugin" }
    async fn before_inference(&self, _ctx: &ReadOnlyContext<'_>) -> ActionSet<BeforeInferenceAction> {
        ActionSet::single(BeforeInferenceAction::AddContextMessage(
            tirea_contract::runtime::inference::ContextMessage {
                key: "time".into(),
                content: "Time: now".into(),
                cooldown_turns: 0,
                target: Default::default(),
            },
        ))
            .and(BeforeInferenceAction::ExcludeTool("dangerous_tool".into()))
    }
}
```

Registry types follow the rename:

| 0.2 | 0.3 |
|-----|-----|
| `AgentPlugin` | `AgentBehavior` |
| `PluginRegistry` | `BehaviorRegistry` |
| `InMemoryPluginRegistry` | `InMemoryBehaviorRegistry` |
| `builder.with_registered_plugin(...)` | `builder.with_registered_behavior(...)` |
| `AgentDefinition.plugin_ids` | `AgentDefinition.behavior_ids` |

New `AgentBehavior` methods (all have default implementations):

- `behavior_ids()` — returns list of behavior IDs (default: `vec![self.id()]`)
- `register_lattice_paths(registry)` — register CRDT merge paths
- `register_state_scopes(registry)` — register state scope metadata
- `register_state_action_deserializers(registry)` — register action deserialization

### RunConfig → RunPolicy

The JSON-bag `RunConfig` (alias for `SealedState`) is replaced by a strongly-typed `RunPolicy`.

```rust,ignore
// Before (0.2)
pub type RunConfig = tirea_state::SealedState;
// Access: run_config.value("allowed_tools")

// After (0.3)
pub struct RunPolicy {
    allowed_tools: Option<Vec<String>>,
    excluded_tools: Option<Vec<String>>,
    allowed_skills: Option<Vec<String>>,
    excluded_skills: Option<Vec<String>>,
    allowed_agents: Option<Vec<String>>,
    excluded_agents: Option<Vec<String>>,
}
// Access: run_policy.allowed_tools()
```

| 0.2 | 0.3 |
|-----|-----|
| `StepContext::run_config()` | `ReadOnlyContext::run_policy()` |
| `ToolExecutionRequest` field `run_config` | field `run_policy` |
| `RunContext::new(..., run_config)` | `RunContext::new(..., run_policy)` |

### Module Restructuring

The `orchestrator` module in `tirea-agentos` is split into `composition` and `runtime`.

```rust,ignore
// Before (0.2)
use tirea_agentos::orchestrator::{AgentOsBuilder, AgentDefinition, AgentOs};
use tirea::orchestrator::{AgentOsBuilder, AgentDefinition};

// After (0.3)
use tirea_agentos::composition::{AgentOsBuilder, AgentDefinition};
use tirea_agentos::runtime::{AgentOs, RunStream};
// or via umbrella:
use tirea::composition::{AgentOsBuilder, AgentDefinition};
use tirea::runtime::{AgentOs, RunStream};
```

`AgentOs`, `AgentOsBuilder`, and `AgentDefinition` are also re-exported at the crate root
(`tirea_agentos::AgentOs`, etc.).

### Phase Context Module Move

```rust,ignore
// Before (0.2)
use tirea_contract::runtime::plugin::phase::{Phase, StepContext, ...};
use tirea_contract::runtime::plugin::AgentPlugin;

// After (0.3)
use tirea_contract::runtime::phase::{Phase, ActionSet, BeforeInferenceAction, ...};
use tirea_contract::runtime::behavior::AgentBehavior;
```

### Action Trait and ToolExecutionEffect

Tools can now return typed state actions via `execute_effect()`:

```rust,ignore
// After (0.3)
async fn execute_effect(&self, args: Value, ctx: &ToolCallContext<'_>)
    -> Result<ToolExecutionEffect, ToolError>
{
    let effect = ToolExecutionEffect::new(ToolResult::success(json!("done")))
        .with_action(AnyStateAction::new::<MyState>(MyAction::Increment));
    Ok(effect)
}
```

The default `execute_effect()` delegates to `execute()`, so existing tools work unchanged.

### StateSpec and State Scopes

New `StateSpec` trait for typed state with reducer pattern:

```rust,ignore
#[derive(State)]
#[tirea(path = "__my_state", action = "MyAction", scope = "run")]
pub struct MyState { pub count: i64 }

impl MyState {
    fn reduce(&mut self, action: MyAction) {
        match action {
            MyAction::Increment => self.count += 1,
        }
    }
}
```

`StateScope` controls cleanup lifecycle:

| Scope | Lifetime |
|-------|----------|
| `Thread` (default) | Persists across runs |
| `Run` | Deleted at start of each new run |
| `ToolCall` | Scoped to `__tool_call_scope.<call_id>`, cleaned after call completes |

### CRDT / Lattice Support

New lattice system for conflict-free replicated state:

- `#[derive(Lattice)]` proc-macro
- `#[tirea(lattice)]` field attribute on `#[derive(State)]` structs
- `Op::LatticeMerge` operation variant
- `LatticeRegistry` for merge dispatch
- Primitives: `Flag`, `MaxReg`, `MinReg`, `GCounter`, `GSet`, `ORSet`, `ORMap`

### New State Trait Methods

`State` gains three new methods (all have default implementations):

```rust,ignore
fn register_lattice(_registry: &mut LatticeRegistry) {}
fn lattice_keys() -> &'static [&'static str] { &[] }
fn diff_ops(old: &Self, new: &Self, base_path: &Path) -> TireaResult<Vec<Op>> { ... }
```

Existing `#[derive(State)]` types remain compatible.

### Sub-Agent System Redesign

`Delegation*` types are renamed to `SubAgent*`. Child threads are now stored independently
in `ThreadStore` rather than embedded in parent state.

| 0.2 | 0.3 |
|-----|-----|
| `DelegationStatus` | `SubAgentStatus` |
| `DelegationRecord` | `SubAgent` (lightweight metadata) |
| `DelegationState` | `SubAgentState` |
| `AgentRunManager` | `SubAgentHandleTable` |
| State path `agent_runs` | State path `sub_agents` |
| `DelegationRecord.thread` (embedded) | Independent `ThreadStore` entry at `sub-agent-{run_id}` |

### Plugin Extension Context Changes

Permission and reminder context extension traits are replaced by typed actions:

```rust,ignore
// Before (0.2)
ctx.allow_tool("tool_name");       // PermissionContextExt
ctx.add_reminder("text");          // ReminderContextExt

// After (0.3) — use typed actions via ToolExecutionEffect
ToolExecutionEffect::new(result)
    .with_action(permission_state_action(PermissionAction::SetTool { ... }))
    .with_action(add_reminder_action("text"))
```

| 0.2 | 0.3 |
|-----|-----|
| `PermissionContextExt` | `PermissionAction` + `permission_state_action()` |
| `ReminderContextExt` | `ReminderAction` + `add_reminder_action()` / `clear_reminder_action()` |
| `SkillPlugin` | `SkillDiscoveryPlugin` |

### Type Renames

| 0.2 | 0.3 |
|-----|-----|
| `RunState` | `RunLifecycleState` (with `#[tirea(scope = "run")]`) |
| `ToolCallStatesMap` | Removed — `ToolCallState` is now per-call scoped |
| `SuspendedToolCallsState` | `SuspendedCallState` (per-call scoped) |
| `InferenceErrorState` | Removed — errors carried in `LLMResponse` |
| `TerminationReason::PluginRequested` | `TerminationReason::BehaviorRequested` |

### StreamResult Gains StopReason

```rust,ignore
pub struct StreamResult {
    pub text: String,
    pub tool_calls: Vec<ToolCall>,
    pub usage: Option<TokenUsage>,
    pub stop_reason: Option<StopReason>,  // new
}

pub enum StopReason { EndTurn, MaxTokens, ToolUse, StopSequence }
```

### ToolResult.suspension Boxing

```rust,ignore
// Before (0.2)
pub suspension: Option<SuspendTicket>,

// After (0.3)
pub suspension: Option<Box<SuspendTicket>>,
```

### state_paths Module Removed

```rust,ignore
// Before (0.2)
use tirea_contract::runtime::state_paths::{RUN_LIFECYCLE_STATE_PATH, ...};

// After (0.3) — use PATH constants from state types directly
RunLifecycleState::PATH  // "__run"
```

### Context Window Management

New plugin-based context window management (optional):

```rust,ignore
use tirea_agentos::runtime::ContextPlugin;

let plugin = ContextPlugin::for_model("claude");
builder.with_registered_behavior("context_window", Arc::new(plugin));
```

### declare_plugin_states! Macro

Replaces `impl_loop_config_builder_methods!`. Generates `register_lattice_paths`,
`register_state_scopes`, and `register_state_action_deserializers` implementations
for `AgentBehavior` types.

### ThreadReader New Methods

`ThreadReader` gains run query methods (all have default implementations):

- `load_run(run_id)` — load a single run record
- `list_runs(query)` — paginated run listing with filters
- `active_run_for_thread(thread_id)` — find the active run for a thread

## 0.1 → 0.2

### Module Reorganization

The `event` and `protocol` modules have been consolidated into a unified `io` module.

```rust,ignore
// Before (0.1)
use tirea_contract::event::AgentEvent;
use tirea_contract::protocol::{ProtocolInputAdapter, ProtocolOutputEncoder, ProtocolHistoryEncoder};

// After (0.2)
use tirea_contract::io::{AgentEvent, RunRequest, RuntimeInput, ToolCallDecision};
use tirea_contract::transport::{Transcoder, Identity};
```

The three protocol traits (`ProtocolInputAdapter`, `ProtocolOutputEncoder`, `ProtocolHistoryEncoder`)
are replaced by a single `Transcoder` trait. Use `Identity<T>` as a pass-through when no
transformation is needed.

### Plugin Interface

The single `on_phase(Phase, StepContext)` method is replaced by individual per-phase methods,
each with a typed context parameter.

```rust,ignore
// Before (0.1)
#[async_trait]
impl AgentPlugin for MyPlugin {
    fn id(&self) -> &str { "my_plugin" }
    async fn on_phase(&self, phase: Phase, ctx: &mut StepContext<'_, '_>) {
        match phase {
            Phase::BeforeInference => { /* ... */ }
            Phase::AfterInference => { /* ... */ }
            _ => {}
        }
    }
}

// After (0.2)
#[async_trait]
impl AgentPlugin for MyPlugin {
    fn id(&self) -> &str { "my_plugin" }
    async fn before_inference(&self, ctx: &mut BeforeInferenceContext<'_, '_>) {
        ctx.add_system_context("Time: now".into());
    }
    async fn after_inference(&self, ctx: &mut AfterInferenceContext<'_, '_>) {
        // ...
    }
}
```

Available phase methods: `run_start`, `step_start`, `before_inference`, `after_inference`,
`before_tool_execute`, `after_tool_execute`, `step_end`, `run_end`. All have empty default
implementations.

Other plugin API changes:

| 0.1 | 0.2 |
|-----|-----|
| `ctx.skip_inference()` | `ctx.terminate_plugin_requested()` |
| `ctx.ask_frontend_tool(...)` | Removed — use `SuspendTicket` via `BeforeToolExecuteContext` |
| `ctx.ask(...)` / `ctx.allow(...)` / `ctx.deny(...)` | Removed — use block/allow/suspend on `BeforeToolExecuteContext` |
| `StepContext` skip/termination fields | Removed — use `RunAction` / `ToolCallAction` on typed contexts |

### Event Stream Changes

Removed variants:

| Removed | Replacement |
|---------|-------------|
| `AgentEvent::InteractionRequested` | Tool-call suspension emitted via `ToolCallDone` with pending status |
| `AgentEvent::InteractionResolved` | `AgentEvent::ToolCallResumed { target_id, result }` |
| `AgentEvent::Pending` | Run lifecycle uses `TerminationReason::Suspended` |

New variants:

- `AgentEvent::ReasoningDelta { delta }` — streaming reasoning content.
- `AgentEvent::ReasoningEncryptedValue { encrypted_value }` — encrypted reasoning block.
- `AgentEvent::ToolCallResumed { target_id, result }` — resume decision applied.
- `AgentEvent::InferenceComplete` now includes `duration_ms: u64`.

### Suspension Model

The single-slot "pending interaction" model is replaced by per-call tool-call suspension.

```rust,ignore
// Before (0.1)
let pending = ctx.pending_interaction();
let frontend = ctx.pending_frontend_invocation();

// After (0.2)
let suspended: &HashMap<String, SuspendedCall> = ctx.suspended_calls();
```

Key type changes:

| 0.1 | 0.2 |
|-----|-----|
| `ToolSuspension` | `SuspendTicket` (deprecated alias provided) |
| `pending_interaction()` | `suspended_calls()` |
| `pending_frontend_invocation()` | Removed |
| Single pending slot | Per-call `SuspendedCall` map |
| Resume via outbox replay | Resume via `ToolCallDecision` on decision channel |

Each `SuspendedCall` carries: `call_id`, `tool_name`, `arguments`, and a `ticket: SuspendTicket`
field that holds the `suspension` payload, `pending` projection, and `resume_mode`
(ReplayToolCall / UseDecisionAsToolResult / PassDecisionToTool).

### Type Renames

Deprecated aliases are provided for all renames. Update imports to suppress warnings:

| 0.1 | 0.2 |
|-----|-----|
| `RunLifecycleStatus` | `RunStatus` |
| `RunLifecycleAction` | `RunAction` (flow control: Continue/Terminate) |
| `ToolCallLifecycleAction` | `ToolCallAction` (gate: Proceed/Suspend/Block) |
| `ToolCallLifecycleState` | `ToolCallState` |
| `ToolSuspension` | `SuspendTicket` |

### Method Renames

Deprecated forwarding methods are provided. Update call sites to suppress warnings:

| 0.1 | 0.2 |
|-----|-----|
| `StateManager::apply(patch)` | `StateManager::commit(patch)` |
| `StateManager::apply_batch(patches)` | `StateManager::commit_batch(patches)` |

### Stop Conditions

Stop conditions moved from core config to `StopPolicyPlugin`. If you were passing stop conditions
to `BaseAgent`, register them as a plugin instead:

```rust,ignore
use tirea::composition::{AgentDefinition, StopConditionSpec};

// Stop conditions are now declared on AgentDefinition:
let agent = AgentDefinition::new("deepseek-chat")
    .with_stop_condition_specs(vec![
        StopConditionSpec::MaxRounds { rounds: 10 },
        StopConditionSpec::Timeout { seconds: 300 },
    ]);
// AgentOsBuilder wires the stop_policy behavior automatically
```

### `RunRequest` Changes

`RunRequest` now requires `initial_decisions` and optionally accepts `parent_run_id`:

```rust,ignore
// Before (0.1)
let req = RunRequest {
    agent_id: "agent_1".into(),
    thread_id: Some("thread_1".into()),
    run_id: None,
    resource_id: None,
    state: None,
    messages: vec![user_message],
};

// After (0.2)
let req = RunRequest {
    agent_id: "agent_1".into(),
    thread_id: Some("thread_1".into()),
    run_id: None,
    parent_run_id: None,  // new
    parent_thread_id: None,
    resource_id: None,
    origin: RunOrigin::default(),
    state: None,
    messages: vec![user_message],
    initial_decisions: vec![],  // new, required
};
```

### Durable State Paths

New durable state paths persisted in thread state:

| Path | Type | Content |
|------|------|---------|
| `__run` | `RunLifecycleState` | Run id, status, done_reason, updated_at |
| `__tool_call_scope.<call_id>.tool_call_state` | `ToolCallState` | Per-call lifecycle status |
| `__tool_call_scope.<call_id>.suspended_call` | `SuspendedCallState` | Suspended call payload |

### Deleted Crates

- `tirea-interaction-plugin` — functionality absorbed into core suspension model.

## Pre-0.1 → 0.1

### Terminology Updates

- `Session` -> `Thread`
- Storage traits remain `ThreadReader` / `ThreadWriter` / `ThreadStore`
- Session routes -> `/v1/threads` routes

### Runtime Surface Updates

- Prefer `AgentOs::run_stream(RunRequest)` for app-level integration.
- Use `RunContext` as run-scoped mutable workspace.
- Use protocol adapters (`AG-UI`, `AI SDK v6`) for transport-specific request/response mapping.
