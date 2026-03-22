# Add a Plugin

Use this for cross-cutting behavior such as policy checks, approval gates, reminders, and observability.

## Prerequisites

- You know which phase should emit behavior (`RunStart`, `BeforeInference`, `BeforeToolExecute`, `AfterToolExecute`, `RunEnd`, etc.).
- Plugin side effects are explicit and bounded.

## Steps

1. Implement `AgentBehavior` and assign a stable `id()`.
2. Optionally declare ordering via `fn ordering(&self) -> PluginOrdering`.
3. Optionally implement `fn configure(&self, config: &mut AgentRunConfig)` for one-time setup before the loop starts.
4. Return phase actions with `ActionSet<...>` from the phase hooks you need.
5. Register behavior in `AgentOsBuilder::with_registered_behavior("id", plugin)`.
6. Attach behavior id in `AgentDefinition.behavior_ids` or `with_behavior_id(...)`.

## Minimal Pattern

```rust,ignore
use async_trait::async_trait;
use tirea::contracts::runtime::inference::ContextMessage;
use tirea::contracts::runtime::phase::{ActionSet, BeforeInferenceAction};
use tirea::contracts::{AgentBehavior, ReadOnlyContext};

struct AuditBehavior;

#[async_trait]
impl AgentBehavior for AuditBehavior {
    fn id(&self) -> &str {
        "audit"
    }

    async fn before_inference(
        &self,
        _ctx: &ReadOnlyContext<'_>,
    ) -> ActionSet<BeforeInferenceAction> {
        ActionSet::single(BeforeInferenceAction::AddContextMessage(
            ContextMessage::new("audit", "Audit: request entering inference"),
        ))
    }
}
```

## Plugin Ordering

When a plugin must run after (or before) another, override `ordering()`:

```rust,ignore
use tirea::contracts::runtime::behavior::PluginOrdering;

fn ordering(&self) -> PluginOrdering {
    PluginOrdering::after(&["permission"])
}
```

`PluginOrdering` supports `after`, `before`, or both. Plugins without ordering constraints run in registration order.

## Configuration Hook

`configure` runs once during resolve, before the agent loop starts. Use it to seed `AgentRunConfig` with plugin-specific defaults:

```rust,ignore
use tirea::contracts::runtime::run::config::AgentRunConfig;

fn configure(&self, config: &mut AgentRunConfig) {
    config.extensions_mut().insert(MyPluginConfig::default());
}
```

## Injecting Messages After Tool Execution

Use `AfterToolExecuteAction::AddMessage(ContextMessage)` to append a message after a tool result:

```rust,ignore
use tirea::contracts::runtime::inference::ContextMessage;
use tirea::contracts::runtime::phase::{ActionSet, AfterToolExecuteAction};

async fn after_tool_execute(
    &self,
    _ctx: &ReadOnlyContext<'_>,
) -> ActionSet<AfterToolExecuteAction> {
    ActionSet::single(AfterToolExecuteAction::AddMessage(
        ContextMessage::system_reminder("Remember to verify output."),
    ))
}
```

## Verify

- Behavior hook runs at the intended phase.
- Event/thread output contains expected behavior side effects.
- Runs are unchanged when behavior preconditions are not met.
- If ordering is declared, verify the plugin executes in the correct relative order.

## Common Errors

- Registering behavior but forgetting to include its id in `AgentDefinition.behavior_ids`.
- Using the wrong phase (effect appears too early or too late).
- Unbounded mutations in a behavior, making runs hard to reason about.
- Referencing a non-existent plugin id in `PluginOrdering::after` or `::before`.

## Related Example

- `examples/src/travel.rs` shows a production `LLMMetryPlugin` registration path
- `examples/src/starter_backend/mod.rs` wires permission and tool-policy behaviors into multiple agents

## Key Files

- `crates/tirea-contract/src/runtime/behavior.rs`
- `crates/tirea-agentos/src/composition/builder.rs`
- `crates/tirea-agentos/src/runtime/prompt_segments/plugin.rs`
- `crates/tirea-extension-permission/src/plugin.rs`

## Related

- [Actions](../reference/actions.md)
- [Run Lifecycle and Phases](../explanation/run-lifecycle-and-phases.md)
- [Build an Agent](./build-an-agent.md)
- [Debug a Run](./debug-a-run.md)
