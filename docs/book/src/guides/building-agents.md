# Building Agents

This guide walks through building a complete agent with `AgentOs`.

## Overview

An agent requires:

1. **Tools** — Functions the LLM can call
2. **Agent definition** — Model, system prompt, tools, stop conditions
3. **AgentOs** — Wiring layer that connects everything
4. **Thread store** — Thread persistence

## Step 1: Define Tools

```rust,ignore
use tirea::{Tool, ToolDescriptor, ToolResult, ToolError, Context};
use async_trait::async_trait;
use serde_json::{json, Value};

struct SearchTool;

#[async_trait]
impl Tool for SearchTool {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor::new("search", "Search", "Search for information")
            .with_parameters(json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "Search query" }
                },
                "required": ["query"]
            }))
    }

    async fn execute(&self, args: Value, ctx: &Context<'_>) -> Result<ToolResult, ToolError> {
        let query = args["query"].as_str().unwrap_or("");
        // ... perform search ...
        Ok(ToolResult::success("search", json!({"results": []})))
    }
}
```

## Step 2: Build AgentOs

```rust,ignore
use tirea::{
    AgentOs, AgentOsBuilder, AgentDefinition, AgentConfig,
    MemoryStore, MaxRounds, StopConditionSpec,
};
use std::sync::Arc;

let os = AgentOsBuilder::new()
    // Register tools
    .tool("search", Arc::new(SearchTool))
    // Register agent
    .agent("assistant", AgentDefinition {
        model: "gpt-4".to_string(),
        system_prompt: Some("You are a helpful assistant.".to_string()),
        tools: vec!["search".to_string()],
        config: AgentConfig {
            stop_conditions: vec![StopConditionSpec::MaxRounds(MaxRounds::new(10))],
            ..Default::default()
        },
        ..Default::default()
    })
    .build()?;
```

## Step 3: Run the Agent

### Blocking (returns final session)

```rust,ignore
use tirea::{run_loop, Session, Message, RunContext};

let session = Session::new("session-1")
    .with_message(Message::user("Search for Rust async patterns"));

let run_ctx = RunContext::new(&os, "assistant");
let result = run_loop(run_ctx, session).await?;
```

### Streaming (returns event stream)

```rust,ignore
use tirea::run_loop_stream;
use futures::StreamExt;

let stream = run_loop_stream(run_ctx, session);

while let Some(event) = stream.next().await {
    match event {
        AgentEvent::TextDelta(text) => print!("{}", text),
        AgentEvent::ToolCall { name, .. } => println!("[calling {}]", name),
        AgentEvent::Done(session) => break,
        _ => {}
    }
}
```

## Step 4: Persist Threads

```rust,ignore
use tirea::{ThreadReader, ThreadWriter};
use tirea_store_adapters::FileStore;

let thread_store = FileStore::new("./threads");

// Save
thread_store.save(&thread).await?;

// Load
let loaded = thread_store.load_thread("thread-1").await?;
```

## Plugins

Add cross-cutting behavior with plugins:

```rust,ignore
let os = AgentOsBuilder::new()
    .tool("search", Arc::new(SearchTool))
    .plugin("metrics", Arc::new(LLMMetryPlugin::new(InMemorySink::new())))
    .plugin("permissions", Arc::new(PermissionPlugin::default()))
    .agent("assistant", /* ... */)
    .build()?;
```

Plugins hook into the [Phase system](../concepts/agent-framework.md) to intercept events at each stage of the agent loop.

## Protocol Adapters

To serve agents over HTTP with SSE:

- **AI SDK v6** — Use `run_ai_sdk_sse` for Vercel AI SDK compatible streaming
- **AG-UI** — Use `run_agent_stream_sse` for CopilotKit compatible streaming

See [Server Gateway](../concepts/server-gateway.md) for the full HTTP server setup.
