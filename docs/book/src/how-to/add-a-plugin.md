# Add a Plugin

Use this for cross-cutting behavior such as policy checks, approval gates, and observability.

## Prerequisites

- You know the phase where behavior should happen (`BeforeInference`, `BeforeToolExecute`, `AfterToolExecute`).
- Plugin side effects are explicit and bounded.

## Steps

1. Implement `AgentPlugin` and give it a unique `id()`.
2. Handle only the phases relevant to your concern.
3. Register plugin via `AgentOsBuilder::with_registered_plugin("id", plugin)`.
4. Attach plugin id in `AgentDefinition::with_plugin_id(...)` or `plugin_ids`.

## Minimal Pattern

```rust,ignore
use async_trait::async_trait;
use tirea::contracts::{AgentPlugin, BeforeInferenceContext};

struct AuditPlugin;

#[async_trait]
impl AgentPlugin for AuditPlugin {
    fn id(&self) -> &str {
        "audit"
    }

    async fn before_inference(&self, ctx: &mut BeforeInferenceContext<'_, '_>) {
        ctx.add_system_context("Audit: request entering inference".to_string());
    }
}
```

## Verify

- Plugin hook runs at intended phase.
- Event/thread output contains expected plugin side effects.
- Normal runs are unchanged when plugin condition is not met.

## Common Errors

- Using wrong phase causes missing or late behavior.
- Plugin id not registered or not attached to the agent.
- Plugin mutates too much state, making runs hard to reason about.

## Related

- [Run Lifecycle and Phases](../explanation/run-lifecycle-and-phases.md)
- [Build an Agent](./build-an-agent.md)
- [Debug a Run](./debug-a-run.md)
