# Add a Tool

Use this when you already have an agent and need to add one tool safely.

If your tool arguments have a stable Rust shape, start with [`TypedTool`](../reference/typed-tool.md). Reach for plain `Tool` only when you need manual JSON handling or custom effect wiring.

## Prerequisites

- Existing `AgentOsBuilder` wiring.
- Tool behavior can be expressed as one deterministic unit of work.
- You know whether this tool should be exposed to the model (`allowed_tools`).

## Steps

1. Choose `TypedTool` for fixed schemas, or `Tool` for dynamic/manual schemas.
2. Implement the tool with a stable descriptor id (`tool_id()` for `TypedTool`, `descriptor().id` for `Tool`).
3. Validate arguments explicitly (`validate()` for `TypedTool`, `execute` or `validate_args` for `Tool`).
4. Keep execution deterministic on the same `(args, state)` when possible.
5. Register tool with `AgentOsBuilder::with_tools(...)`.
6. If using whitelist mode, include tool id in `AgentDefinition::with_allowed_tools(...)`.

## Preferred Pattern: `TypedTool`

```rust,ignore
use async_trait::async_trait;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::json;
use tirea::contracts::runtime::tool_call::{ToolError, ToolResult, TypedTool};
use tirea::contracts::ToolCallContext;

#[derive(Debug, Deserialize, JsonSchema)]
struct MyToolArgs {
    input: String,
}

struct MyTool;

#[async_trait]
impl TypedTool for MyTool {
    type Args = MyToolArgs;

    fn tool_id(&self) -> &str { "my_tool" }
    fn name(&self) -> &str { "My Tool" }
    fn description(&self) -> &str { "Do one thing" }

    fn validate(&self, args: &Self::Args) -> Result<(), String> {
        if args.input.trim().is_empty() {
            return Err("input cannot be empty".to_string());
        }
        Ok(())
    }

    async fn execute(
        &self,
        args: MyToolArgs,
        _ctx: &ToolCallContext<'_>,
    ) -> Result<ToolResult, ToolError> {
        Ok(ToolResult::success("my_tool", json!({ "input": args.input })))
    }
}
```

## Alternative Pattern: plain `Tool`

Use this when argument shape is dynamic or you need manual JSON handling.

```rust,ignore
use async_trait::async_trait;
use serde_json::{json, Value};
use tirea::contracts::ToolCallContext;
use tirea::prelude::{Tool, ToolDescriptor, ToolError, ToolResult};

struct MyUntypedTool;

#[async_trait]
impl Tool for MyUntypedTool {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor::new("my_untyped_tool", "My Untyped Tool", "Do one thing")
            .with_parameters(json!({
                "type": "object",
                "properties": { "input": { "type": "string" } },
                "required": ["input"]
            }))
    }

    async fn execute(&self, args: Value, _ctx: &ToolCallContext<'_>) -> Result<ToolResult, ToolError> {
        let input = args["input"]
            .as_str()
            .ok_or_else(|| ToolError::InvalidArguments("input is required".to_string()))?;
        Ok(ToolResult::success("my_untyped_tool", json!({ "input": input })))
    }
}
```

## Verify

- Run event stream includes `ToolCallStart` and `ToolCallDone` for `my_tool`.
- Thread message history includes tool call + tool result messages.
- If tool writes state, a new patch is appended.

## State Access Checklist

- Prefer `ctx.state_of::<T>()` when the state type has a canonical `#[tirea(path = "...")]`.
- Use `ctx.state::<T>("custom.path")` when the same state shape is reused at a different path.
- Put durable business data in `thread` scope, temporary run coordination in `run`, and per-call scratch data in `tool_call`.
- If one tool depends on another tool having run first, encode that precondition in state and reject invalid execution explicitly.
- If the tool must return a result and also emit side effects such as state actions or message injection, implement `execute_effect` instead of stopping at `ToolResult`.

For concrete examples, see [Typed Tool](../reference/typed-tool.md).

## Common Errors

- Descriptor id mismatch: registration and `allowed_tools` must use identical id.
- Missing derives for `TypedTool`: `Args` must implement both `Deserialize` and `JsonSchema`.
- Silent argument defaults: prefer explicit validation for required fields.
- Non-deterministic side effects: hard to replay/debug and can break tests.
- Choosing plain `Tool` for a fixed schema: this usually adds parsing noise and drifts schema away from Rust types.

## Related Example

- `examples/ai-sdk-starter/README.md` is the shortest end-to-end path for adding tools to a browser demo
- `examples/copilotkit-starter/README.md` shows tool rendering, approval, and persisted-thread integration

## Key Files

- `examples/src/starter_backend/tools.rs`
- `examples/src/travel/tools.rs`
- `examples/src/research/tools.rs`
- `crates/tirea-contract/src/runtime/tool_call/tool.rs`

## Related

- [First Tool](../tutorials/first-tool.md)
- [Typed Tool](../reference/typed-tool.md)
- [Build an Agent](./build-an-agent.md)
- [Errors](../reference/errors.md)
