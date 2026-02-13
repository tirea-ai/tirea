# Getting Started

This guide walks you through the basics of `carve-state` and `carve-agent`.

## Prerequisites

Add the dependencies to your `Cargo.toml`:

```toml
[dependencies]
carve-state = { version = "0.1", features = ["derive"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
```

For agent functionality:

```toml
[dependencies]
carve-agent = "0.1"
tokio = { version = "1", features = ["full"] }
async-trait = "0.1"
```

## Step 1: Working with Patches

The simplest way to use `carve-state` is with raw JSON patches:

```rust
use carve_state::{apply_patch, Patch, Op, path};
use serde_json::json;

// Start with initial state
let state = json!({"count": 0, "name": "counter"});

// Build a patch
let patch = Patch::new()
    .with_op(Op::set(path!("count"), json!(10)))
    .with_op(Op::set(path!("updated"), json!(true)));

// Apply patch — pure function, original unchanged
let new_state = apply_patch(&state, &patch).unwrap();

assert_eq!(new_state["count"], 10);
assert_eq!(state["count"], 0);
```

## Step 2: Using JsonWriter

`JsonWriter` provides a builder-style API for constructing patches:

```rust
use carve_state::{JsonWriter, path};
use serde_json::json;

let mut w = JsonWriter::new();
w.set(path!("user", "name"), json!("Alice"));
w.append(path!("user", "roles"), json!("admin"));
w.increment(path!("user", "login_count"), 1i64);

let patch = w.build();
// patch contains 3 operations
```

## Step 3: Typed State with Derive

For type safety, use `#[derive(State)]`:

```rust,ignore
use carve_state::State;
use carve_state_derive::State;
use serde::{Serialize, Deserialize};

#[derive(Debug, Clone, Serialize, Deserialize, State)]
struct Counter {
    value: i64,
    label: String,
}
```

This generates `CounterRef<'a>` with getters (`value()`, `label()`) and setters (`set_value()`, `set_label()`). See the [Derive Macro Reference](./derive-macro.md) for all attributes.

## Step 4: Implementing a Tool

Tools are the primary way agents interact with state:

```rust,ignore
use carve_agent::{Tool, ToolDescriptor, ToolResult, ToolError, Context};
use async_trait::async_trait;
use serde_json::{json, Value};

struct IncrementTool;

#[async_trait]
impl Tool for IncrementTool {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor::new("increment", "Increment", "Increment a counter")
            .with_parameters(json!({
                "type": "object",
                "properties": {
                    "amount": { "type": "integer", "default": 1 }
                }
            }))
    }

    async fn execute(&self, args: Value, ctx: &Context<'_>) -> Result<ToolResult, ToolError> {
        let amount = args["amount"].as_i64().unwrap_or(1);
        let counter = ctx.state::<Counter>("counter");
        let current = counter.value().map_err(|e| ToolError::execution(e.to_string()))?;
        counter.set_value(current + amount);

        Ok(ToolResult::success("increment", json!({"new_value": current + amount})))
    }
}
```

## Next Steps

- [Derive Macro Reference](./derive-macro.md) — All `#[carve(...)]` attributes
- [Building Agents](./building-agents.md) — Full agent setup with AgentOs
- [Operations](../reference/operations.md) — All patch operation types
