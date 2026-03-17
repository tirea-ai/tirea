# Actions

This page lists the action types available in the runtime, what tools can emit, what plugins can emit, and which built-in plugins use them.

## Why This Matters

Tirea is not a "tool returns result only" runtime.

- tools can emit `ToolExecutionEffect`
- plugins return `ActionSet<...>` from phase hooks
- the loop validates actions by phase, applies them to `StepContext`, and reduces state actions into patches

Use this page when you need to answer:

- "Can a tool change state or only return `ToolResult`?"
- "Which action type is valid in this phase?"
- "Should this behavior live in a tool or a plugin?"

## Core Phase Actions

The authoritative definitions live in [`crates/tirea-contract/src/runtime/phase/action_set.rs`](/home/chaizhenhua/Codes/uncarve/crates/tirea-contract/src/runtime/phase/action_set.rs).

| Phase action enum | Valid phase | What it can do | Typical use |
|---|---|---|---|
| `LifecycleAction` | `RunStart`, `StepStart`, `StepEnd`, `RunEnd` | `State(AnyStateAction)` | lifecycle bookkeeping, run metadata |
| `BeforeInferenceAction` | `BeforeInference` | `AddContextMessage`, `AddSessionContext`, `ExcludeTool`, `IncludeOnlyTools`, `AddRequestTransform`, `Terminate`, `State` | prompt injection, tool filtering, context-window shaping, early termination |
| `AfterInferenceAction` | `AfterInference` | `Terminate`, `State` | inspect model response and stop or persist derived state |
| `BeforeToolExecuteAction` | `BeforeToolExecute` | `Block`, `Suspend`, `SetToolResult`, `State` | permission checks, frontend approval, short-circuiting tool execution |
| `AfterToolExecuteAction` | `AfterToolExecute` | `AddSystemReminder`, `AddUserMessage`, `State` | append follow-up context, inject skill instructions, persist post-tool state |

## What Tools Can Emit

Tools do not directly return phase enums like `BeforeInferenceAction`. A tool can influence the runtime in three ways:

1. Return `ToolResult`
2. Write state through `ToolCallContext`
3. Return `ToolExecutionEffect` with `Action`s

In practice, a tool can safely do:

- direct typed state writes through `ctx.state_of::<T>()` or `ctx.state::<T>(...)`
- explicit `AnyStateAction`
- built-in `AfterToolExecuteAction`
- custom `Action` implementations only when no built-in `AfterToolExecuteAction` variant matches

The most common tool-side actions are:

- `AnyStateAction`
- `AfterToolExecuteAction::AddUserMessage`
- `AfterToolExecuteAction::AddSystemReminder`
- custom actions that mutate step-local runtime data after a tool completes and have no built-in equivalent

Important constraint:

- tools run inside tool execution, so they should think in `AfterToolExecute` terms
- they should not try to behave like `BeforeInference` or `BeforeToolExecute` plugins

## `AnyStateAction`

`AnyStateAction` is the generic state mutation wrapper. It is the main bridge between reducer-backed state and the action pipeline.

Use it when:

- you want reducer-style typed state updates instead of direct setter-style writes
- a tool or plugin needs to mutate typed state in a phase-aware way
- you need thread/run/tool-call scope to be resolved consistently by the runtime

Common constructors:

- `AnyStateAction::new::<T>(action)` for thread/run scoped state
- `AnyStateAction::new_for_call::<T>(action, call_id)` for tool-call scoped state

Important:

- direct `ctx.state...` writes are materialized into patches during tool execution and folded into the same effect pipeline
- for business code, prefer `ctx.state...` for straightforward typed updates or `AnyStateAction::new...` for reducer-style domain actions

## What Plugins Can Emit

Plugins emit phase-specific core action enums through `ActionSet<...>`.

Typical patterns:

- `BeforeInferenceAction` for prompt/context/tool selection shaping
- `BeforeToolExecuteAction` for gating, approval, or short-circuiting tools
- `AfterToolExecuteAction` for injecting reminders/messages after a tool
- `LifecycleAction` for lifecycle-scoped state changes

If a behavior must apply uniformly across many tools or every run, it belongs in a plugin.

## Built-in Plugin Action Matrix

### Public extension plugins

| Plugin | Actions used | Scenario |
|---|---|---|
| `ReminderPlugin` | `BeforeInferenceAction::AddSessionContext`, `BeforeInferenceAction::State` | inject reminder text into the next inference and optionally clear reminder state |
| `PermissionPlugin` | `BeforeToolExecuteAction::Block`, `BeforeToolExecuteAction::Suspend` | deny a tool or suspend for permission approval |
| `ToolPolicyPlugin` | `BeforeInferenceAction::IncludeOnlyTools`, `BeforeInferenceAction::ExcludeTool`, `BeforeToolExecuteAction::Block` | constrain visible tools up front and enforce scope at execution time |
| `SkillDiscoveryPlugin` | `BeforeInferenceAction::AddContextMessage` | inject the active skill catalog or skill usage instructions into the prompt |
| `LLMMetryPlugin` | no runtime-mutating actions; returns empty `ActionSet` | observability only, collects spans and metrics without changing behavior |

### Built-in runtime / integration plugins

| Plugin | Actions used | Scenario |
|---|---|---|
| `ContextPlugin` | `BeforeInferenceAction::AddRequestTransform` | compact + trim history and enable prompt caching before the provider request is sent |
| `AG-UI ContextInjectionPlugin` | `BeforeInferenceAction::AddContextMessage` | inject frontend-provided context into the prompt |
| `AG-UI FrontendToolPendingPlugin` | `BeforeToolExecuteAction::Suspend`, `BeforeToolExecuteAction::SetToolResult` | forward frontend tools to the UI, then resume with a frontend decision/result |

## Tool Examples That Emit Actions

### Skill activation tool

`SkillActivateTool` is the clearest example of a tool returning more than a result.

It emits:

- a success `ToolResult`
- `AnyStateAction` for `SkillStateAction::Activate(...)`
- permission-domain state actions via `permission_state_action(...)`
- `AfterToolExecuteAction::AddUserMessage` to insert skill instructions into the message stream

See:

- [`crates/tirea-extension-skills/src/tools.rs`](/home/chaizhenhua/Codes/uncarve/crates/tirea-extension-skills/src/tools.rs)
- [`crates/tirea-extension-permission/src/state.rs`](/home/chaizhenhua/Codes/uncarve/crates/tirea-extension-permission/src/state.rs)

### Direct state writes in tools

When a tool writes state through `ToolCallContext`, the runtime collects the resulting patch and turns it into an action-backed execution effect during tool execution.

That means both of these are valid:

- write via `ctx.state_of::<T>()`
- emit explicit `AnyStateAction`

Choose based on which model fits the state update better:

- direct field/setter patching for straightforward state edits
- reducer action form when you want a domain action log or reducer semantics

Direct `ctx.state...` writes produce patches that the runtime handles internally. Business code should use either `ctx.state...` setters or `AnyStateAction::new...` constructors.

## Guidance By Scenario

| Scenario | Recommended action form |
|---|---|
| Add prompt context before the next model call | `BeforeInferenceAction::AddContextMessage` or `AddSessionContext` |
| Hide or narrow tools for one run | `BeforeInferenceAction::IncludeOnlyTools` / `ExcludeTool` |
| Enforce approval before a tool executes | `BeforeToolExecuteAction::Suspend` |
| Reject tool execution with an explicit reason | `BeforeToolExecuteAction::Block` |
| Return a synthetic tool result without running the tool | `BeforeToolExecuteAction::SetToolResult` |
| Persist typed state from a tool or plugin | `AnyStateAction` or direct `ctx.state...` writes |
| Add follow-up instructions/messages after a tool completes | `AfterToolExecuteAction::AddUserMessage` |
| Modify request assembly itself | `BeforeInferenceAction::AddRequestTransform` |

## Rule Of Thumb

- If you need phase-aware orchestration, use a plugin and core phase actions.
- If you need domain work plus a result plus post-tool side effects, use a tool with `execute_effect`.
- If all you need is state mutation, stay on the typed path: `ctx.state...` for direct updates or `AnyStateAction::new...` for reducer-style actions.

## Related

- [Tool and Plugin Boundary](../explanation/tool-and-plugin-boundary.md)
- [Typed Tool](./typed-tool.md)
- [Add a Plugin](../how-to/add-a-plugin.md)
- [Run Lifecycle and Phases](../explanation/run-lifecycle-and-phases.md)
