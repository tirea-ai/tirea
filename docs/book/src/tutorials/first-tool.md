# First Tool

## Goal

Implement one tool that reads and updates typed state.

## Prerequisites

- Complete [First Agent](./first-agent.md) first.
- `State` derive is available in your dependencies:

```toml
[dependencies]
serde = { version = "1", features = ["derive"] }
tirea-state-derive = "0.1"
```

## 1. Define Typed State

```rust,ignore
use serde::{Deserialize, Serialize};
use tirea_state_derive::State;

#[derive(Debug, Clone, Serialize, Deserialize, State)]
struct Counter {
    value: i64,
    label: String,
}
```

## 2. Implement the Tool

```rust,ignore
use async_trait::async_trait;
use serde_json::{json, Value};
use tirea::contracts::ToolCallContext;
use tirea::prelude::*;

struct IncrementCounter;

#[async_trait]
impl Tool for IncrementCounter {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor::new("increment_counter", "Increment Counter", "Increment counter state")
            .with_parameters(json!({
                "type": "object",
                "properties": {
                    "amount": { "type": "integer", "default": 1 }
                }
            }))
    }

    async fn execute(&self, args: Value, ctx: &ToolCallContext<'_>) -> Result<ToolResult, ToolError> {
        let amount = args["amount"].as_i64().unwrap_or(1);
        let counter = ctx.state::<Counter>("counter");
        let current = counter.value().unwrap_or(0);
        counter.set_value(current + amount);

        Ok(ToolResult::success(
            "increment_counter",
            json!({ "before": current, "after": current + amount }),
        ))
    }
}
```

## 3. Register the Tool

```rust,ignore
use tirea::orchestrator::AgentOsBuilder;
use tirea::runtime::loop_runner::tool_map;

let os = AgentOsBuilder::new()
    .with_tools(tool_map([IncrementCounter]))
    .build()?;
```

## 4. Verify Behavior

Run one request that triggers `increment_counter`, then verify:

- Event stream contains `ToolCallDone` for `increment_counter`
- Thread state `counter.value` increases by expected amount
- Thread patch history appends at least one new patch

## Common Errors

- Missing derive macro import: ensure `use tirea_state_derive::State;` exists.
- Wrong state path: `ctx.state::<Counter>("counter")` must match where you read/write.
- Numeric parse fallback hides bugs: validate `amount` if strict input is required.

## Next

- [Build an Agent](../how-to/build-an-agent.md)
- [Add a Tool](../how-to/add-a-tool.md)
- [State Ops Reference](../reference/state-ops.md)
