# Add a Tool

Use this when you already have an agent and need to add one tool safely.

## Prerequisites

- Existing `AgentOsBuilder` wiring.
- Tool behavior can be expressed as one deterministic unit of work.
- You know whether this tool should be exposed to the model (`allowed_tools`).

## Steps

1. Implement `Tool` with a stable `descriptor().id`.
2. Validate arguments in `execute`; return `ToolError::InvalidArguments` for malformed inputs.
3. Keep `execute` deterministic on the same `(args, state)` when possible.
4. Register tool with `AgentOsBuilder::with_tools(...)`.
5. If using whitelist mode, include tool id in `AgentDefinition::with_allowed_tools(...)`.

## Minimal Pattern

```rust,ignore
use async_trait::async_trait;
use serde_json::{json, Value};
use tirea::contracts::{Tool, ToolCallContext, ToolDescriptor, ToolError, ToolResult};

struct MyTool;

#[async_trait]
impl Tool for MyTool {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor::new("my_tool", "My Tool", "Do one thing")
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
        Ok(ToolResult::success("my_tool", json!({ "input": input })))
    }
}
```

## Verify

- Run event stream includes `ToolCallStart` and `ToolCallDone` for `my_tool`.
- Thread message history includes tool call + tool result messages.
- If tool writes state, a new patch is appended.

## Common Errors

- Descriptor id mismatch: registration and `allowed_tools` must use identical id.
- Silent argument defaults: prefer explicit validation for required fields.
- Non-deterministic side effects: hard to replay/debug and can break tests.

## Related

- [First Tool](../tutorials/first-tool.md)
- [Build an Agent](./build-an-agent.md)
- [Errors](../reference/errors.md)
