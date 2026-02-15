# Agent Framework

`carve-agent` provides a framework for building AI agents where all state changes are tracked through patches.

## Architecture

```text
┌─────────────────────────────────────────────────────┐
│  Application Layer                                   │
│  - Register tools, run agent loop, persist sessions  │
└─────────────────────────────────────────────────────┘
                         │
                         ▼
┌─────────────────────────────────────────────────────┐
│  Pure Functions                                      │
│  - build_request, StreamCollector, Session::with_*  │
└─────────────────────────────────────────────────────┘
                         │
                         ▼
┌─────────────────────────────────────────────────────┐
│  Tool Layer                                          │
│  - Tool trait, Context, automatic patch collection   │
└─────────────────────────────────────────────────────┘
```

## Core Types

### Tool

The `Tool` trait is the primary extension point. Each tool provides:

- **`descriptor()`** — Name, display name, description, and JSON Schema parameters
- **`execute(args, ctx)`** — Async execution with typed state access via `Context`

```rust,ignore
use carve_agent::{Tool, ToolDescriptor, ToolResult, ToolError, Context};
use async_trait::async_trait;
use serde_json::{json, Value};

struct CalculatorTool;

#[async_trait]
impl Tool for CalculatorTool {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor::new("calculator", "Calculator", "Evaluate expressions")
            .with_parameters(json!({
                "type": "object",
                "properties": { "expr": { "type": "string" } },
                "required": ["expr"]
            }))
    }

    async fn execute(&self, args: Value, _ctx: &Context<'_>) -> Result<ToolResult, ToolError> {
        let expr = args["expr"].as_str().unwrap_or("0");
        Ok(ToolResult::success("calculator", json!({"result": 42})))
    }
}
```

### Session

An immutable conversation state containing messages and patch history:

```rust,ignore
let session = Session::new("session-1")
    .with_message(Message::user("Hello"))
    .with_message(Message::assistant("Hi!"));
```

Sessions are immutable — `with_*` methods return new sessions.

### Agent Loop

The agent loop orchestrates the LLM interaction cycle:

```text
┌──────────────────────────────────────────┐
│  run_loop / run_loop_stream              │
│                                          │
│  for each step:                          │
│    1. StepStart phase                    │
│    2. BeforeInference → build request    │
│    3. LLM call → stream response         │
│    4. AfterInference → process response  │
│    5. BeforeToolExecute                  │
│    6. Execute tools → collect patches    │
│    7. AfterToolExecute                   │
│    8. StepEnd → check stop conditions    │
└──────────────────────────────────────────┘
```

### Plugins (Phase System)

`AgentPlugin` hooks into the loop at specific phases:

- `SessionStart` / `SessionEnd` — Once per session
- `StepStart` / `StepEnd` — Each loop iteration
- `BeforeInference` / `AfterInference` — Around LLM calls
- `BeforeToolExecute` / `AfterToolExecute` — Around tool execution

Built-in plugins: `PermissionPlugin`, `ReminderPlugin`, `LLMMetryPlugin`.

### Stop Conditions

Control when the agent loop terminates:

- `MaxRounds` — Maximum loop iterations
- `Timeout` — Wall-clock time limit
- `TokenBudget` — Token usage limit
- `StopOnTool` — Stop when a specific tool is called
- `ContentMatch` — Stop on matching content
- `ConsecutiveErrors` — Stop after repeated failures
- `LoopDetection` — Detect repetitive behavior

## AgentOs

`AgentOs` is the top-level wiring layer that brings everything together:

- **Agent Registry** — Named agent definitions (model, tools, config)
- **Tool Registry** — Named tool implementations
- **Model Registry** — Model definitions and provider configs
- **Plugin Registry** — Shared plugin instances
- **Provider Registry** — Context providers for system prompts
- **Skill Subsystem** — Optional skill discovery and runtime

Use `AgentOsBuilder` to configure and build an `AgentOs` instance.

## Thread Store

Thread persistence is modeled with:

- `ThreadReader` / `ThreadWriter` traits
- `ThreadStore` (combined read+write trait)
- `MemoryStore` — In-memory (for testing)
- `carve_thread_store_adapters::FileStore` — JSON files on disk
- `carve_thread_store_adapters::PostgresStore` — PostgreSQL (`postgres` feature)
- `carve_thread_store_adapters::NatsBufferedThreadWriter` — NATS JetStream buffering decorator (`nats` feature)

## Streaming

`StreamCollector` collects streaming LLM responses, and `AgentEvent` provides a unified event stream for the agent loop. Protocol adapters:

- `AiSdkAdapter` — AI SDK v6 compatible SSE
- `AgUiAdapter` — AG-UI / CopilotKit compatible events
