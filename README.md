# Awaken

**A production-ready AI agent runtime for Rust — type-safe state, phase-based execution, multi-protocol serving.**

Build production AI agents with compile-time guarantees, deterministic phase execution, and built-in observability. Define your agent logic once and serve it over AI SDK, AG-UI, A2A, and MCP from a single binary.

> **Note:** Awaken is a ground-up rewrite of [tirea](../../tree/tirea-0.5), redesigned for simplicity and production reliability. The tirea 0.5 codebase is archived on the [`tirea-0.5`](../../tree/tirea-0.5) branch for reference. Awaken is **not** backwards-compatible with tirea — see the [migration guide](./docs/book/src/appendix/migration-from-tirea.md).

## 30-second mental model

1. **Tools** — typed functions your agent can call; JSON schema is generated at compile time
2. **Agents** — each agent has a system prompt, a model, and a set of allowed tools; the LLM drives orchestration through natural language — no predefined graphs
3. **State** — typed, scoped (thread / run / tool-call), with merge strategies for safe concurrent writes and immutable snapshots
4. **Plugins** — lifecycle hooks for permissions, observability, context management, skills, MCP, and more

Your agent picks tools, calls them, reads and updates state, and repeats — all orchestrated by the runtime through 8 typed phases. Every state change is committed atomically after the gather phase.

## Why Awaken

| What you get | How it works |
|---|---|
| **Ship one backend for every frontend** | Serve React (AI SDK v6), Next.js (AG-UI), other agents (A2A), and tool servers (MCP) from the same binary. No separate deployments. |
| **LLM orchestrates everything — no DAGs** | Define each agent's identity and tool access; the LLM decides when to delegate, to whom, and how to combine results. No hand-coded graphs or state machines. |
| **Type-safe state with scoping and replay** | State is a Rust struct with compile-time checks. Merge strategies handle concurrent writes without locks. Scope to thread, run, or tool-call. Every change is an immutable snapshot you can replay. |
| **Catch plugin wiring errors at compile time** | Plugins hook into 8 typed lifecycle phases. Wire a permission check to the wrong phase? The compiler tells you, not your users. |
| **Production resilience built in** | Circuit breaker, exponential backoff, inference timeout, graceful shutdown, Prometheus metrics, and health probes — all included. |
| **Zero `unsafe` code** | The entire workspace forbids `unsafe`. Memory safety is guaranteed by the Rust compiler. |

### Feature comparison

|  | Awaken | LangGraph | AG2 | CrewAI | OpenAI Agents | Mastra |
|---|:---:|:---:|:---:|:---:|:---:|:---:|
| **Language** | Rust | Python/TS | Python | Python | Python/TS | TypeScript |
| **Orchestration** | Tool delegation | Stateful graph | Conversational | Role-based | Handoffs + as_tool | Workflow + LLM |
| **Multi-protocol server** | AG-UI · AI SDK · A2A · MCP | ◐ | ◐ | ◐ | ❌ | AG-UI · AI SDK · A2A |
| **Typed state** | ✅ scoping + merge + replay | ◐ | ❌ | ◐ | ❌ | ◐ |
| **Plugin lifecycle** | 8 typed phases | Middleware | ◐ | ◐ | Guardrails | ◐ |
| **Agent handoff** | ✅ | ❌ | ❌ | ❌ | ✅ | ❌ |
| **Per-inference override** | ✅ model + temperature | ❌ | ❌ | ❌ | ❌ | ❌ |
| **Sub-agents** | ✅ | ✅ | ✅ group chat | ✅ | ✅ | ✅ |
| **MCP support** | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| **Human-in-the-loop** | ✅ durable mailbox | ✅ | ✅ | ✅ | ✅ | ✅ |
| **Observability** | ✅ OpenTelemetry | ✅ LangSmith | ✅ OpenTelemetry | ◐ | ✅ | ◐ |
| **Persistence** | ✅ file + postgres | ✅ | ◐ | ◐ | ◐ | ✅ |
| **Circuit breaker** | ✅ per-model | ❌ | ❌ | ❌ | ❌ | ❌ |
| **Deferred tool loading** | ✅ probability model | ❌ | ❌ | ❌ | ❌ | ❌ |

✅ = native  ◐ = partial  ❌ = not available

## Quick setup

```bash
git clone https://github.com/AwakenWorks/awaken.git
cd awaken
cargo install lefthook && lefthook install   # git hooks (format, lint, secrets)
cargo build --workspace
```

> **Prerequisites**: Rust 1.93+ (via `rust-toolchain.toml`), an LLM provider API key (OpenAI, Anthropic, or DeepSeek).

## Usage

### Define tools

```rust
use awaken::prelude::*;
use serde::Deserialize;
use serde_json::json;

#[derive(Deserialize, schemars::JsonSchema)]
struct SearchArgs {
    query: String,
    max_results: Option<usize>,
}

struct SearchTool;

#[async_trait]
impl Tool for SearchTool {
    fn descriptor(&self) -> ToolDescriptor {
        // JSON schema for args is set via .with_parameters(); see schemars docs
        ToolDescriptor::new("search", "search", "Search the web for information")
            .with_parameters(schemars::schema_for!(SearchArgs).into())
    }

    async fn execute(&self, args: Value, _ctx: &ToolCallContext) -> Result<ToolOutput, ToolError> {
        let args: SearchArgs = serde_json::from_value(args)
            .map_err(|e| ToolError::InvalidArguments(e.to_string()))?;
        // ... call your search API ...
        Ok(ToolResult::success("search", json!({
            "results": [{"title": "Rust", "url": "https://rust-lang.org"}]
        })).into())
    }
}
```

### Define agents and assemble

```rust
use awaken::prelude::*;
use std::sync::Arc;

// Define agents — each selects which tools it can use
let mut planner = AgentSpec::new("planner")
    .with_model("gpt-4o")
    .with_system_prompt("You are a travel planner. Use search tools to find options.")
    .with_max_rounds(8);
planner.allowed_tools = Some(vec!["search_flights".into(), "search_hotels".into()]);

let researcher = AgentSpec::new("researcher")
    .with_model("deepseek-chat")
    .with_system_prompt("You research destinations and provide summaries.")
    .with_max_rounds(4);

// Assemble into runtime
let runtime = AgentRuntimeBuilder::new()
    .with_agent_spec(planner)
    .with_agent_spec(researcher)
    .with_tool("search_flights", Arc::new(SearchFlightsTool))
    .with_tool("search_hotels", Arc::new(SearchHotelsTool))
    .with_plugin("permission", Arc::new(PermissionPlugin))
    .with_thread_run_store(Arc::new(FileStore::new("./sessions")))
    .build()?;
```

### Connect to any frontend

Start the server, then connect from React, Next.js, or another agent — no code changes:

```rust
let state = AppState::new(runtime, mailbox, store, resolver, ServerConfig::default());
serve(state).await?;
```

| Protocol | Endpoint | Frontend |
|---|---|---|
| AI SDK v6 | `POST /v1/ai-sdk/chat` | React `useChat()` |
| AG-UI | `POST /v1/ag-ui/run` | CopilotKit `<CopilotKit>` |
| A2A | `POST /v1/a2a/tasks/send` | Other agents |
| Native | `POST /v1/threads/:id/runs` | Any HTTP client |
| Health | `GET /health` · `GET /health/live` | Kubernetes probes |
| Metrics | `GET /metrics` | Prometheus |

**React + AI SDK v6:**

```typescript
import { useChat } from "ai/react";

const { messages, input, handleSubmit } = useChat({
  api: "http://localhost:3000/v1/ai-sdk/chat",
});
```

**Next.js + CopilotKit:**

```typescript
import { CopilotKit } from "@copilotkit/react-core";

<CopilotKit runtimeUrl="http://localhost:3000/v1/ag-ui/run">
  <YourApp />
</CopilotKit>
```

### Built-in plugins

| Plugin | What it does | Feature flag |
|---|---|---|
| **Permission** | Allow/Deny/Ask per tool, human-in-the-loop suspension | `permission` |
| **Observability** | OpenTelemetry spans for LLM calls and tool executions | `observability` |
| **MCP** | Connect to MCP servers; tools discovered automatically | `mcp` |
| **Skills** | Discover and activate skill packages from filesystem | `skills` |
| **Reminder** | Persistent context messages that survive across turns | `reminder` |
| **Generative UI** | Declarative UI components sent to frontends (A2UI) | `generative-ui` |
| **Deferred Tools** | Lazy tool loading with ToolSearch and probability model | `deferred-tools` |

### Manage state across conversations

State is typed and scoped to its intended lifetime:

```rust
use awaken::prelude::*;

pub struct UserPreferences;  // persists across all runs in this thread

impl StateKey for UserPreferences {
    const KEY: &'static str = "user.preferences";
    const MERGE: MergeStrategy = MergeStrategy::Exclusive;
    type Value = PreferencesValue;
    type Update = PreferencesValue;

    fn apply(value: &mut PreferencesValue, update: PreferencesValue) {
        *value = update;
    }
}
```

| Scope | Lifetime | Use case |
|---|---|---|
| `Thread` | Persists across all runs | User preferences, conversation memory |
| `Run` (default) | Reset at each run start | Search progress, tool call tracking |
| `ToolCall` | Exists during one tool execution | Temporary workspace |

### Persist conversations

Swap storage backends without changing agent code:

| Backend | Use case |
|---|---|
| `InMemoryStore` | Tests and ephemeral demos |
| `FileStore` | Local development, single-server deployment |
| `PostgresStore` | Production with SQL queries and backups |

## When to use Awaken

- You want a **Rust backend** for AI agents with compile-time safety
- You need to serve **multiple frontend protocols** from one server
- Your tools need to **safely share state** during concurrent execution
- You need **auditable state history** and replay
- You're building for **production** — low memory, no GC, circuit breakers, Prometheus metrics

## When NOT to use Awaken

- You need **built-in file/shell/web tools** out of the box — consider Dify, CrewAI
- You want a **visual workflow builder** — consider Dify, LangGraph Studio
- You want **Python** and rapid prototyping — consider LangGraph, AG2, PydanticAI
- You need **LLM-managed memory** (agent decides what to remember) — consider Letta

## Architecture

```
awaken                   Facade crate with feature flags
  awaken-contract        Types, traits, state model, agent specs
  awaken-runtime         Phase execution engine, plugin system, agent loop
  awaken-server          Axum HTTP server, SSE, protocol adapters, mailbox
  awaken-stores          Memory, file, and PostgreSQL storage backends
  awaken-tool-pattern    Glob/regex tool matching
  awaken-ext-*           Extensions (permission, observability, mcp, skills,
                         reminder, generative-ui, deferred-tools)
```

## Feature flags

All features enabled by default. Use `default-features = false` to opt out.

| Feature | Default | Description |
|---|:---:|---|
| `permission` | yes | Allow / deny / ask tool policies |
| `observability` | yes | OpenTelemetry tracing with GenAI semantic conventions |
| `mcp` | yes | Model Context Protocol client |
| `skills` | yes | YAML skill package discovery and activation |
| `reminder` | yes | Rule-based context message injection |
| `generative-ui` | yes | Server-driven UI components |
| `server` | yes | Multi-protocol HTTP server with mailbox |
| `full` | yes | All of the above |

## Examples

| Example | What it shows | Best for |
|---|---|---|
| [`live_test`](./crates/awaken/examples/live_test.rs) | Basic LLM integration | First steps |
| [`multi_turn`](./crates/awaken/examples/multi_turn.rs) | Multi-turn with persistent threads | Conversation state |
| [`tool_call_live`](./crates/awaken/examples/tool_call_live.rs) | Tool calling with calculator | Tool integration |
| [`ai-sdk-starter`](./examples/ai-sdk-starter/) | React + AI SDK v6 full-stack | Frontend integration |
| [`copilotkit-starter`](./examples/copilotkit-starter/) | Next.js + CopilotKit full-stack | CopilotKit / AG-UI |

```bash
# Runtime examples
export OPENAI_API_KEY=<your-key>
cargo run --package awaken --example multi_turn

# Full-stack demos
cd examples/ai-sdk-starter && npm install && npm run dev
```

## Learning paths

| Goal | Start with | Then |
|---|---|---|
| Build your first agent | [First Agent tutorial](./docs/book/src/tutorials/first-agent.md) | [Build an Agent guide](./docs/book/src/how-to/build-an-agent.md) |
| See a full-stack app | [AI SDK starter](./examples/ai-sdk-starter/) | [CopilotKit starter](./examples/copilotkit-starter/) |
| Explore the API | [Reference docs](./docs/book/src/reference/overview.md) | `cargo doc --workspace --no-deps --open` |
| Contribute | [Contributing guide](./CONTRIBUTING.md) | [Development setup](./DEVELOPMENT.md) |
| Migrate from tirea | [Migration guide](./docs/book/src/appendix/migration-from-tirea.md) | |

## Documentation

| Resource | Description |
|---|---|
| [`docs/book/`](./docs/book/) | User guide — tutorials, how-to, reference, explanation |
| [`docs/adr/`](./docs/adr/) | 19 Architecture Decision Records |
| [`DEVELOPMENT.md`](./DEVELOPMENT.md) | Build, test, and contribution guide |
| [`CONTRIBUTING.md`](./CONTRIBUTING.md) | Contributor guidelines |

## Project status

Awaken is in active development. The API is not yet stable.

| Component | Status |
|---|---|
| Contract types | Stable — breaking changes unlikely |
| Runtime engine | Maturing — API may evolve |
| Server / protocols | Maturing — AI SDK v6 and AG-UI well-tested |
| Storage backends | Stable (memory, file) / beta (PostgreSQL) |
| Extensions | Stable (permission, observability) / beta (others) |

## Contributing

See [CONTRIBUTING.md](./CONTRIBUTING.md). Contributions welcome — especially:

- Additional storage backends (Redis, SQLite)
- Built-in tool implementations (file read/write, web search)
- Token cost tracking and budget enforcement
- Model fallback/degradation chains

## License

Dual-licensed under [MIT](./LICENSE-MIT) or [Apache-2.0](./LICENSE-APACHE).
