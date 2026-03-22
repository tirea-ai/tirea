**English** | [中文](./docs/README.zh-CN.md)

# Tirea

**Type-safe AI agents that handle concurrent state without locks. One binary serves React, Next.js, and other agents over four protocols.**

Define agents, tools, and state in Rust — then serve them to any frontend over AG-UI, AI SDK v6, A2A, and ACP from a single binary. Connect to external tool servers via MCP.

[![Crates.io](https://img.shields.io/crates/v/tirea.svg)](https://crates.io/crates/tirea)
[![docs.rs](https://img.shields.io/docsrs/tirea)](https://docs.rs/tirea)
[![License](https://img.shields.io/crates/l/tirea)](LICENSE-MIT)

<p align="center">
  <img src="./docs/assets/demo.svg" alt="Tirea demo — tool call + LLM streaming" width="800">
</p>

## 30-second mental model

1. **Tools** — typed functions your agent can call; JSON schema is generated from the struct
2. **Agents** — each agent has a system prompt and a set of allowed tools/sub-agents; the LLM drives all orchestration through natural language — no predefined graphs or state machines
3. **State** — typed, scoped (thread / run / tool_call), with CRDT fields for safe concurrent writes
4. **Plugins** — lifecycle hooks for permissions, observability, context window, reminders, and more

Your agent picks tools, calls them, reads and updates state, and repeats — all orchestrated by the runtime. Every state change is an immutable patch you can replay.

## Why Tirea

| What you get | How it works |
|---|---|
| **Ship one backend for every frontend** | Serve React (AI SDK v6), Next.js (AG-UI), other agents (A2A), and stdio-based agents (ACP) from the same binary. No separate deployments. Connect to external tool servers via MCP. |
| **LLM orchestrates everything — no DAGs** | Define each agent's identity and tool access; the LLM decides when to delegate, to whom, and how to combine results. No hand-coded graphs or state machines. |
| **Type-safe state with CRDT, scoping, and replay** | State is a Rust struct with compile-time checks. CRDT fields merge concurrent tool writes without locks. Scope to thread, run, or tool_call to prevent stale data. Every change is an immutable patch you can replay. |
| **Catch plugin wiring errors at compile time** | Plugins hook into 8 typed lifecycle phases. Wire a permission check to the wrong phase? The compiler tells you, not your users. |
| **Run thousands of agents on minimal resources** | No GC pauses. 32 concurrent agents sustain ~1,000 runs/s on mock LLM. (`cargo bench --package tirea-agentos --bench runtime_throughput` to reproduce.) |

### Feature comparison

|  | Tirea | LangGraph | AG2 | CrewAI | OpenAI Agents | Mastra |
|---|:---:|:---:|:---:|:---:|:---:|:---:|
| **Language** | Rust | Python/TS | Python | Python | Python/TS | TypeScript |
| **Orchestration** | Tool delegation | Stateful graph | Conversational | Role-based | Handoffs + as_tool | Workflow + LLM |
| **Multi-protocol server** | AG-UI · AI SDK · A2A · ACP | ◐ | ◐ | ◐ | ❌ | AG-UI · AI SDK · A2A |
| **Typed state** | ✅ CRDT + scoping + replay | ◐ | ❌ | ◐ | ❌ | ◐ |
| **Plugin lifecycle** | 8 typed phases | Middleware | ◐ | ◐ | Guardrails | ◐ |
| **Agent handoff** | ✅ | ❌ | ❌ | ❌ | ✅ | ❌ |
| **Per-inference override** | ✅ model + reasoning | ❌ | ❌ | ❌ | ❌ | ❌ |
| **Sub-agents** | ✅ | ✅ | ✅ group chat | ✅ | ✅ | ✅ |
| **MCP support** | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| **Human-in-the-loop** | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| **Observability** | ✅ OpenTelemetry | ✅ LangSmith | ✅ OpenTelemetry | ◐ | ✅ | ◐ |
| **Persistence** | ✅ | ✅ | ◐ | ◐ | ◐ | ✅ |

✅ = native  ◐ = partial  ❌ = not available

> **What does "Tool delegation" mean?** Tirea follows the [Claude Code](https://github.com/anthropics/claude-code) orchestration pattern: the LLM manages sub-agents through tool calls (`agent_run`, `agent_stop`, `agent_output`) instead of hand-coded graphs or one-way handoffs.
>
> |  | Tool delegation (Tirea) | Handoffs + as_tool (OpenAI) | Conversational (AG2) |
> |---|---|---|---|
> | **Mechanism** | LLM calls tools to spawn, stop, and read sub-agents | Handoffs transfer control; `as_tool()` calls agent and returns result | Agents converse in a group chat with speaker selection |
> | **Parallel execution** | ✅ background mode, multiple sub-agents | ◐ `as_tool()` via async; handoffs are sequential | ❌ sequential turns |
> | **Bidirectional** | ✅ parent reads child output, can stop | ◐ `as_tool()` returns results; handoffs are one-way | ✅ all agents see shared conversation |
> | **Status awareness** | ✅ auto-injected reminder each turn | ❌ no automatic status injection | ✅ via shared chat history |
> | **Agent discovery** | Dynamic catalog rendered in system prompt | Hard-coded `handoffs=[]` in code | Predefined `participants=[]` |

## Quick start

### Prerequisites

- Rust toolchain from [`rust-toolchain.toml`](./rust-toolchain.toml)
- For frontend demos: Node.js 20+ and npm
- One model provider key (OpenAI, DeepSeek, Anthropic, etc.)

### Full-stack demo in 60 seconds

**React + AI SDK v6:**

```bash
git clone https://github.com/tirea-ai/tirea.git && cd tirea
cd examples/ai-sdk-starter && npm install
DEEPSEEK_API_KEY=<your-key> npm run dev
# First run compiles the Rust agent (~1-2 min), then opens http://localhost:3001
```

**Next.js + CopilotKit:**

```bash
cd examples/copilotkit-starter && npm install
cp .env.example .env.local
DEEPSEEK_API_KEY=<your-key> npm run setup:agent && npm run dev
# Open http://localhost:3000
```

### Server only

```bash
export OPENAI_API_KEY=<your-key>
cargo run --package tirea-agentos-server -- --http-addr 127.0.0.1:8080
```

## Usage

### Architecture

```mermaid
graph LR
    subgraph "Your frontends"
        A["React app\n(AI SDK v6)"]
        B["Next.js app\n(CopilotKit / AG-UI)"]
        C["Another agent\n(A2A)"]
    end

    subgraph "tirea server (one binary)"
        GW["Protocol gateway\nUI: AG-UI · AI SDK\nAgent: A2A · ACP"]
        RT["Agent runtime\nLLM streaming · tool dispatch\nplugin lifecycle · context mgmt"]
        EXT["Extensions\npermission · skills · MCP\nreminder · observability"]
    end

    subgraph "Storage (pick one)"
        S1[(File)]
        S2[(PostgreSQL)]
    end

    A & B --> GW
    C --> GW
    GW --> RT
    RT --> EXT
    RT --> S1 & S2
```

### Define tools, agents, and assemble

```rust
// 1. Build tools — define args as a struct, schema is generated automatically
#[derive(Deserialize, JsonSchema)]
struct SearchFlightsArgs {
    from: String,
    to: String,
    date: String,
}

struct SearchFlightsTool;

#[async_trait]
impl TypedTool for SearchFlightsTool {
    type Args = SearchFlightsArgs;
    fn tool_id(&self) -> &str { "search_flights" }
    fn name(&self) -> &str { "Search Flights" }
    fn description(&self) -> &str { "Find flights between two cities." }

    async fn execute(&self, args: SearchFlightsArgs, _ctx: &ToolCallContext<'_>)
        -> Result<ToolResult, ToolError>
    {
        // ... call your flight API ...
        Ok(ToolResult::success("search_flights", json!({
            "flights": [{"airline": "UA", "price": 342, "from": args.from, "to": args.to}]
        })))
    }
}

// 2. Define agents — each agent selects which tools/skills/sub-agents it can use
let planner = AgentDefinition::with_id("planner", "deepseek-chat")
    .with_system_prompt("You are a travel planner. Use search tools to find options.")
    .with_max_rounds(8)
    .with_allowed_tools(vec!["search_flights".into(), "search_hotels".into()])
    .with_allowed_agents(vec!["researcher".into()]);

let researcher = AgentDefinition::with_id("researcher", "deepseek-chat")
    .with_system_prompt("You research destinations and provide summaries.")
    .with_max_rounds(4)
    .with_excluded_tools(vec!["delete_account".into()]);

// 3. Assemble into AgentOs — the container for all components
let os = AgentOsBuilder::new()
    .with_tools(tool_map_from_arc(vec![
        Arc::new(SearchFlightsTool),
        Arc::new(SearchHotelsTool),
    ]))
    .with_agent_spec(AgentDefinitionSpec::local(planner))
    .with_agent_spec(AgentDefinitionSpec::local(researcher))
    .with_agent_state_store(Arc::new(FileStore::new("./sessions")))
    .build()?;
```

Tools are registered globally. Each agent controls its own access via `allowed_*` / `excluded_*` lists — the runtime filters the tool pool at resolve time.

### Connect to any frontend

Start the server, then connect from React, Next.js, or another agent — no code changes between them:

```bash
cargo run --package tirea-agentos-server -- --http-addr 127.0.0.1:8080
```

| Protocol | Endpoint | Frontend |
|---|---|---|
| AI SDK v6 | `POST /v1/ai-sdk/agents/:agent_id/runs` | React `useChat()` |
| AG-UI | `POST /v1/ag-ui/agents/:agent_id/runs` | CopilotKit `<CopilotKit>` |
| A2A | `POST /v1/a2a/agents/:agent_id/message:send` | Other agents |
| ACP | stdio (stdin/stdout) | CLI and stdio-based agent clients |

**React + AI SDK v6:**

```typescript
import { useChat } from "ai/react";

const { messages, input, handleSubmit } = useChat({
  api: "http://localhost:8080/v1/ai-sdk/agents/assistant/runs",
});
```

**Next.js + CopilotKit:**

```typescript
import { CopilotKit } from "@copilotkit/react-core";

<CopilotKit runtimeUrl="http://localhost:8080/v1/ag-ui/agents/assistant/runs">
  <YourApp />
</CopilotKit>
```

### Built-in tools

Tirea ships with tools for sub-agents, background tasks, skills, UI rendering, and MCP integration. They're auto-registered when you enable the corresponding feature:

| Tool group | Tools | What they do |
|---|---|---|
| **Sub-agents** (core) | `agent_run`, `agent_stop`, `agent_output` | Launch, cancel, and read results from child agents running in parallel |
| **Background tasks** (core) | `task_status`, `task_cancel`, `task_output` | Monitor and manage long-running background operations |
| **Skills** (`skills` feature) | `skill`, `load_skill_resource`, `skill_script` | Discover, activate, and execute skill packages |
| **A2UI** (`a2ui` extension) | `render_a2ui` | Send declarative UI components to the frontend |
| **MCP** (`mcp` feature) | *dynamic* | Tools from connected MCP servers appear as native tools; prompts surface as skills, resources appear in the catalog |

### Require approval before dangerous actions

The built-in `PermissionPlugin` gates tool execution via Allow/Deny/Ask policies per tool. When a tool requires approval, the runtime suspends execution and sends the pending call to the frontend. When the user approves, the runtime replays the original tool call. See the [human-in-the-loop guide](https://tirea-ai.github.io/tirea/explanation/human-in-the-loop.html) for details.

### Multi-agent collaboration

Tirea agents delegate through **natural-language orchestration**. You define each agent's identity and access policy, then register them in the agent registry; the LLM decides when to delegate, to whom, and how to combine results — no DAGs, no hand-coded state machines, no explicit routing logic.

The runtime makes this work:
- **Agent registry** — register agents at build time; the runtime renders the registry into the system prompt so the LLM always knows who it can delegate to
- **Background execution with completion notifications** — sub-agents and tasks run in the background; the runtime injects their status after each tool call, so the LLM stays aware of what's running, what's finished, and what failed
- **Foreground and background modes** — block until a sub-agent finishes, or run multiple sub-agents concurrently in the background and receive completion notifications when each one finishes
- **Thread isolation** — each sub-agent runs in its own thread with independent state
- **Orphan recovery** — if the parent process crashes, orphaned sub-agents are detected and resumed on restart
- **Local + remote transparency** — in-process agents and remote A2A agents use the same `agent_run` interface; the orchestrator doesn't need to know the difference

Register agents at build time:

```rust
let orchestrator = AgentDefinition::with_id("orchestrator", "deepseek-chat")
    .with_system_prompt("Route tasks to the right agent.")
    .with_allowed_agents(vec!["researcher".into(), "writer".into()]);

let researcher = AgentDefinition::with_id("researcher", "deepseek-chat")
    .with_system_prompt("Research topics and return summaries.")
    .with_excluded_tools(vec!["agent_run".into()]); // no further delegation

let os = AgentOsBuilder::new()
    .with_agent_spec(AgentDefinitionSpec::local(orchestrator))
    .with_agent_spec(AgentDefinitionSpec::local(researcher))
    // Remote agents via A2A protocol
    .with_agent_spec(AgentDefinitionSpec::a2a_with_id(
        "writer",
        A2aAgentBinding::new("https://writer-service.example.com/v1/a2a", "writer-v2"),
    ))
    .build()?;
```

See the [multi-agent design patterns guide](https://tirea-ai.github.io/tirea/explanation/multi-agent-design-patterns.html) for coordinator, pipeline, fan-out, and other patterns.

### Manage state across conversations

State is typed and scoped to its intended lifetime:

```rust
#[derive(State)]
#[tirea(scope = "thread")]   // persists across all runs in this conversation
struct UserPreferences { /* ... */ }

#[derive(State)]
#[tirea(scope = "run")]      // reset at the start of each agent run
struct SearchProgress { /* ... */ }

#[derive(State)]
#[tirea(scope = "tool_call")] // exists only during a single tool execution
struct ToolWorkspace { /* ... */ }
```

Fields marked `#[tirea(lattice)]` use CRDT types (conflict-free replicated data types) that merge automatically when parallel tool calls write concurrently — no locks needed. Non-CRDT fields are guarded by conflict detection.

### Persist conversations

Swap storage backends without changing agent code:

| Backend | Use case |
|---|---|
| `FileStore` | Local development, single-server deployment |
| `PostgresStore` | Production with SQL queries and backups |
| `MemoryStore` | Tests |

### Extend with plugins

Plugins hook into 8 lifecycle phases. Use built-in plugins or write your own:

| Plugin | What it does | How to enable |
|---|---|---|
| **Context** | Token budget, message summarization, prompt caching | `ContextPlugin::for_model("claude-sonnet-4-20250514")` |
| **Stop Policy** | Terminate on max rounds, timeout, token budget, loop detection | `StopPolicyPlugin::new(conditions, specs)` |
| **Permission** | Allow/Deny/Ask per tool, human-in-the-loop suspension | `PermissionPlugin` + `ToolPolicyPlugin` |
| **Skills** | Discover and activate skill packages from filesystem | `skills` feature flag |
| **MCP** | Connect to MCP servers; tools, prompts, and resources are discovered automatically | `mcp` feature flag |
| **Reminder** | Persistent system reminders that survive across turns | `add_reminder_action()` helper |
| **Observability** | OpenTelemetry spans for LLM calls and tool executions | `LLMMetryPlugin::new(sink)` |
| **A2UI** | Declarative UI components sent to the frontend | `A2uiPlugin::with_catalog_id(url)` |
| **Handoff** | Dynamic same-thread agent switching at runtime | `HandoffPlugin` + `handoff` feature flag |
| **Prompt Segments** | Unified prompt injection for system context, reminders, and skill instructions | Auto-wired by runtime |
| **Agent Recovery** | Detect and resume orphaned sub-agent runs | Auto-wired with sub-agents |
| **Background Tasks** | Track and inject background task status | Auto-wired with task tools |

### Use any LLM provider

Powered by [genai](https://crates.io/crates/genai) — works with OpenAI, Anthropic, DeepSeek, Google, Mistral, Groq, Ollama, and more. Switch providers by changing one string:

```rust
model: "gpt-4o".into(),        // OpenAI
model: "deepseek-chat".into(), // DeepSeek
model: "claude-sonnet-4-20250514".into(), // Anthropic
```

## When to use Tirea

- You want a **Rust backend** for AI agents with compile-time safety
- You need to serve **multiple frontend protocols** from one server
- Your tools need to **safely share state** during concurrent execution
- You need **auditable state history** and replay
- You're building for **production** — low memory, no GC, thousands of concurrent agents

## When NOT to use Tirea

- You need **built-in file/shell/web tools** out of the box — consider Dify, CrewAI
- You want a **visual workflow builder** — consider Dify, LangGraph Studio
- You want **Python** and rapid prototyping — consider LangGraph, AG2, PydanticAI
- You need **LLM-managed memory** (agent decides what to remember) — consider Letta

## Design inspirations

Tirea's architecture draws from ideas across several projects:

- **Claude Code** — natural-language multi-agent orchestration, mailbox-based message passing, persistent reminders across turns
- **ag2 / FastAgency** — NATS JetStream as a durable transport layer for agent event streaming and crash recovery
- **LangGraph** — typed state with reducers and checkpoint persistence (Tirea extends this with patch-based immutability and replay)

## Learning paths

| Goal | Start with | Then |
|---|---|---|
| Build your first agent | [First Agent tutorial](https://tirea-ai.github.io/tirea/tutorials/first-agent.html) | [Build an Agent guide](https://tirea-ai.github.io/tirea/how-to/build-an-agent.html) |
| See a full-stack app | [AI SDK starter](./examples/ai-sdk-starter/README.md) | [CopilotKit starter](./examples/copilotkit-starter/README.md) |
| Explore the API | [API reference](https://tirea-ai.github.io/tirea/reference/api.html) | `cargo doc --workspace --no-deps --open` |
| Contribute | [Contributing guide](./CONTRIBUTING.md) | [Capability matrix](https://tirea-ai.github.io/tirea/reference/capability-matrix.html) |

## Examples

| Example | What it shows | Best for |
|---|---|---|
| [ai-sdk-starter](./examples/ai-sdk-starter/) | React + AI SDK v6 — chat, canvas, shared state | Fastest start, minimal setup |
| [copilotkit-starter](./examples/copilotkit-starter/) | Next.js + CopilotKit — persisted threads, frontend actions | Full-stack with persistence |
| [travel-ui](./examples/travel-ui/) | Map canvas + approval-gated trip planning | Geospatial + human-in-the-loop |
| [research-ui](./examples/research-ui/) | Resource collection + report writing with approval | Approval-gated workflows |

## Documentation

Full book: <https://tirea-ai.github.io/tirea/> · [API reference](https://docs.rs/tirea) · [Book source](./docs/book/src/)

## Contributing

See [CONTRIBUTING.md](./CONTRIBUTING.md). Contributions welcome — especially:

- Built-in tool implementations (file read/write, search, shell execution)
- Tool-level concurrency safety flags
- Model fallback/degradation chains
- Token cost tracking
- Additional storage backends

## License

Dual-licensed under [MIT](./LICENSE-MIT) or [Apache-2.0](./LICENSE-APACHE).

[SECURITY.md](./SECURITY.md) · [CODE_OF_CONDUCT.md](./CODE_OF_CONDUCT.md)
