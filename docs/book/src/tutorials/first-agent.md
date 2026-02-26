# First Agent

## Goal

Run one agent end-to-end and confirm you receive a complete event stream.

## Prerequisites

```toml
[dependencies]
tirea = "0.2"
tokio = { version = "1", features = ["full"] }
async-trait = "0.1"
futures = "0.3"
serde_json = "1"
```

Set one model provider key before running:

```bash
# OpenAI-compatible models (for gpt-4o-mini)
export OPENAI_API_KEY=<your-key>

# Or DeepSeek models
export DEEPSEEK_API_KEY=<your-key>
```

## 1. Create `src/main.rs`

```rust,ignore
use async_trait::async_trait;
use futures::StreamExt;
use serde_json::{json, Value};
use tirea::contracts::{AgentEvent, Message, RunRequest, ToolCallContext};
use tirea::orchestrator::{AgentDefinition, AgentOsBuilder};
use tirea::runtime::loop_runner::tool_map;

struct EchoTool;

#[async_trait]
impl tirea::contracts::Tool for EchoTool {
    fn descriptor(&self) -> tirea::contracts::ToolDescriptor {
        tirea::contracts::ToolDescriptor::new("echo", "Echo", "Echo input")
            .with_parameters(json!({
                "type": "object",
                "properties": { "text": { "type": "string" } },
                "required": ["text"]
            }))
    }

    async fn execute(
        &self,
        args: Value,
        _ctx: &ToolCallContext<'_>,
    ) -> Result<tirea::contracts::ToolResult, tirea::contracts::ToolError> {
        let text = args["text"].as_str().unwrap_or_default();
        Ok(tirea::contracts::ToolResult::success("echo", json!({ "text": text })))
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let os = AgentOsBuilder::new()
        .with_tools(tool_map([EchoTool]))
        .with_agent(
            "assistant",
            AgentDefinition::with_id("assistant", "gpt-4o-mini")
                .with_system_prompt("You are a helpful assistant.")
                .with_allowed_tools(vec!["echo".to_string()]),
        )
        .build()?;

    let run = os
        .run_stream(RunRequest {
            agent_id: "assistant".to_string(),
            thread_id: Some("thread-1".to_string()),
            run_id: None,
            parent_run_id: None,
            resource_id: None,
            state: None,
            messages: vec![Message::user("Say hello using the echo tool")],
            initial_decisions: vec![],
        })
        .await?;

    let events: Vec<_> = run.events.collect().await;
    println!("events: {}", events.len());

    let finished = events.iter().any(|e| matches!(e, AgentEvent::RunFinish { .. }));
    println!("run_finish_seen: {}", finished);

    Ok(())
}
```

## 2. Run

```bash
cargo run
```

## 3. Verify

Expected output includes:

- `events: <n>` where `n > 0`
- `run_finish_seen: true`

## Common Errors

- Model/provider mismatch: `gpt-4o-mini` requires a compatible OpenAI-style provider setup.
- Missing key: set `OPENAI_API_KEY` or `DEEPSEEK_API_KEY` before `cargo run`.
- Tool not selected: ensure prompt explicitly asks to use `echo`.

## Next

- [First Tool](./first-tool.md)
- [Build an Agent](../how-to/build-an-agent.md)
- [Events Reference](../reference/events.md)
