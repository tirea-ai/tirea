# Awaken

[English](./README.md) | [中文](./docs/book/README.zh-CN.md)

**A high-performance AI agent runtime for Rust — build once, serve everywhere, extend through plugins.**

Define your agent's identity and tools, let the LLM orchestrate everything, and serve it over multiple protocols from a single binary. Extend agent behavior through a composable plugin system with compile-time safety.

> **Status:** Awaken is in active development. Contract types are stable; runtime and server APIs may still evolve. See [Project status](#project-status) for details.

> **Note:** Awaken is a ground-up rewrite of [tirea](../../tree/tirea-0.5), redesigned for simplicity and production reliability. The tirea 0.5 codebase is archived on the [`tirea-0.5`](../../tree/tirea-0.5) branch for reference. Awaken is **not** backwards-compatible with tirea — see the [migration guide](./docs/book/src/appendix/migration-from-tirea.md).

## 30-second mental model

1. **Tools** — typed functions your agent can call; JSON schema is generated at compile time
2. **Agents** — each agent has a system prompt, a model, and a set of allowed tools; the LLM drives orchestration through natural language — no predefined graphs
3. **State** — typed and scoped (`thread` / `run`), with merge strategies for safe concurrent writes and immutable snapshots
4. **Plugins** — lifecycle hooks for permissions, observability, context management, skills, MCP, and more

Your agent picks tools, calls them, reads and updates state, and repeats — all orchestrated by the runtime through 8 typed phases. Every state change is committed atomically after the gather phase.

## Quick setup

```bash
git clone https://github.com/AwakenWorks/awaken.git
cd awaken
cargo install lefthook && lefthook install   # git hooks (format, lint, secrets)
cargo build --workspace
```

> **Prerequisites**: Rust toolchain `1.93.0` (pinned in `rust-toolchain.toml`; workspace `rust-version` metadata is `1.85`) and an LLM provider API key (OpenAI, Anthropic, or DeepSeek).

## Usage

### Define a tool

```rust
use awaken::prelude::*;
use awaken::contract::tool::ToolOutput;
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Deserialize, schemars::JsonSchema)]
struct SearchArgs {
    query: String,
    max_results: Option<usize>,
}

struct SearchTool;

#[async_trait]
impl Tool for SearchTool {
    fn descriptor(&self) -> ToolDescriptor {
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

### Assemble agents into a runtime

```rust
use awaken::prelude::*;
use awaken::engine::GenaiExecutor;
use awaken::stores::InMemoryStore;
use std::sync::Arc;

let runtime = AgentRuntimeBuilder::new()
    .with_provider("openai", Arc::new(GenaiExecutor::new()))
    .with_model(
        "gpt-4o-mini",
        ModelSpec {
            id: "gpt-4o-mini".into(),
            provider: "openai".into(),
            model: "gpt-4o-mini".into(),
        },
    )
    .with_tool("search", Arc::new(SearchTool))
    .with_agent_spec(
        AgentSpec::new("researcher")
            .with_model("gpt-4o-mini")
            .with_system_prompt("You research destinations and provide summaries.")
            .with_max_rounds(4),
    )
    .with_thread_run_store(Arc::new(InMemoryStore::new()))
    .build()?;
```

> **Note:** `with_provider()` and `with_model()` wire up the LLM executor; `with_plugin()` registers a plugin; `.with_hook_filter()` on `AgentSpec` activates that plugin for the agent. See [Plugin activation](#plugin-activation-model) for details.

### Connect to any frontend

Start the server, then connect from React, Next.js, or another agent — no code changes:

```rust
use awaken::prelude::*;
use awaken::stores::{InMemoryMailboxStore, InMemoryStore};
use std::sync::Arc;

let store = Arc::new(InMemoryStore::new());
let runtime = Arc::new(runtime);
let mailbox = Arc::new(Mailbox::new(
    runtime.clone(),
    Arc::new(InMemoryMailboxStore::new()),
    "default-consumer".into(),
    MailboxConfig::default(),
));

let state = AppState::new(
    runtime.clone(),
    mailbox,
    store,
    runtime.resolver_arc(),
    ServerConfig::default(),
);
serve(state).await?;
```

#### Frontend protocols

| Protocol | Endpoint | Frontend |
|---|---|---|
| AI SDK v6 | `POST /v1/ai-sdk/chat` | React `useChat()` |
| AG-UI | `POST /v1/ag-ui/run` | CopilotKit `<CopilotKit>` |
| A2A | `POST /v1/a2a/tasks/send` | Other agents |

#### Native HTTP API

| Endpoint | Description |
|---|---|
| `POST /v1/runs` | Start a new run |
| `GET /v1/threads` | List threads |
| `GET /v1/runs/:id` | Get run status |

#### MCP

| Endpoint | Status |
|---|---|
| `POST /v1/mcp` | JSON-RPC 2.0 request/response |
| `GET /v1/mcp` | SSE — not yet implemented (returns 405) |

#### Operational

| Endpoint | Description |
|---|---|
| `GET /health` · `GET /health/live` | Kubernetes probes |
| `GET /metrics` | Prometheus metrics |

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

## Built-in plugins

Awaken extends agent capabilities through a composable plugin system built on 8 typed lifecycle phases.

| Plugin | Description | Feature flag |
|---|---|---|
| **Permission** | Firewall-style tool access control: evaluates Deny > Allow > Ask rules by priority with glob/regex matching on tool names and arguments. Ask policy triggers human-in-the-loop suspension via the mailbox. | `permission` |
| **Reminder** | Rule-based context injection: when a tool call matches a configured pattern and output condition, automatically injects a system/session/conversation-level context message. Supports cooldown to prevent repetition. | `reminder` |
| **Observability** | OpenTelemetry telemetry aligned with GenAI Semantic Conventions: captures per-inference and per-tool metrics, supports in-memory, file, and OTLP export. | `observability` |
| **MCP** | Model Context Protocol client: connects to external MCP servers and automatically discovers and registers their tools as native Awaken tools. | `mcp` |
| **Skills** | Skill package discovery and activation: supports filesystem and compile-time embedded skill sources. Injects a skill catalog before inference so the LLM can select and activate skills on demand. | `skills` |
| **Generative UI** | Server-driven UI rendering: streams declarative UI components to frontends via the A2UI protocol, enabling rich interactive experiences without frontend changes. | `generative-ui` |

The `awaken-ext-deferred-tools` crate provides lazy tool loading with a probability model, but is not exposed through the `awaken` facade crate — depend on it directly if needed.

### Plugin activation model

Enabling a plugin involves three distinct steps:

1. **Cargo feature flag** — controls whether the extension crate is compiled into your binary (all enabled by default via the `full` feature)
2. **`.with_plugin(id, plugin)`** on the builder — registers the plugin in the runtime's plugin registry
3. **`.with_hook_filter(id)`** on `AgentSpec` — activates the plugin's hooks for that specific agent

This means different agents can use different subsets of registered plugins. A plugin registered but not referenced by any agent's hook filter will not execute.

### Reminder: configuration-driven context injection

The Reminder plugin is a key mechanism for managing agent behavior through configuration rather than code. It lets you define declarative rules that automatically inject context messages into conversations based on tool execution patterns.

Each rule has three parts:

- **Trigger pattern** — matches tool name and arguments (e.g., `"Bash"`, `"Edit(file_path ~ '*.toml')"`)
- **Output condition** — matches execution result (success/error/any, or content pattern)
- **Injected message** — a context message at a chosen level (system, suffix_system, session, or conversation)

For example, you can configure a rule so that when the agent runs a `Bash` command that returns an error, a system-level reminder like "check if the command requires sudo" is automatically injected. This means agent behavior policies live in configuration files, not hardcoded in prompts — making them easy to iterate, version, and share across agents.

## Why Awaken

| What you get | How it works |
|---|---|
| **Ship one backend for every frontend** | Serve React (AI SDK v6), Next.js (AG-UI), other agents (A2A), and tool servers (MCP) from the same binary. No separate deployments. |
| **LLM orchestrates everything — no DAGs** | Define each agent's identity and tool access; the LLM decides when to delegate, to whom, and how to combine results. No hand-coded graphs or state machines. |
| **Composable plugin system** | 8 typed lifecycle phases. Permission, context injection, observability, tool discovery — all wired declaratively. Phase hooks are type-safe (`PhaseHook` trait), and the plugin registration API catches misconfigurations at build time. |
| **Type-safe state with scoping and replay** | State is a Rust struct with compile-time checks. Merge strategies handle concurrent writes without locks. Scope to thread or run. Every change is an immutable snapshot you can replay. |
| **Production resilience built in** | Circuit breaker, exponential backoff, inference timeout, graceful shutdown, Prometheus metrics, and health probes — all included. |
| **Zero `unsafe` code** | The entire workspace forbids `unsafe`. Memory safety is guaranteed by the Rust compiler. |

## Compared with other frameworks

The ecosystem moves too quickly for a large feature matrix to stay reliable. A better summary is:

| Framework | Best known for | Compared with Awaken |
|---|---|---|
| **LangGraph** | Python/TS graph orchestration with persistence, memory, and handoffs | Better fit if you want graph-first orchestration in Python. Awaken is stronger when you want a Rust runtime with typed state and a built-in multi-protocol server. |
| **AG2** | Python multi-agent conversations, group chat, and protocol integrations | Better fit if your team wants Python-first conversational agent systems. Awaken is stronger when you want explicit runtime layering, typed state, and durable server-side control. |
| **CrewAI** | Python `Crews + Flows` workflows and operations tooling | Better fit if you want workflow-driven orchestration and Python tooling. Awaken is stronger when you want LLM-led tool orchestration with Rust-level safety and protocol serving from one backend. |
| **OpenAI Agents SDK** | OpenAI-first agent SDK with hosted tools, sessions, MCP, and guardrails | Better fit if you want OpenAI-managed batteries-included tooling. Awaken is stronger when you want provider-neutral runtime composition, your own storage, and your own server surface. |
| **Mastra** | TypeScript full-stack agent apps with AI SDK, MCP, memory, and frontend integration | Better fit if your stack is already TypeScript end to end. Awaken is stronger when the backend must be Rust and runtime concerns need to stay server-side. |

## When to use Awaken

- You want a **Rust backend** for AI agents with compile-time safety
- You need to serve **multiple frontend or agent protocols** from one backend
- Your tools need to **safely share state** during concurrent execution
- You need **auditable thread history**, checkpoints, and resumable control paths
- You are comfortable wiring your own tools, providers, and model registry instead of relying on batteries-included defaults

## When NOT to use Awaken

- You need **built-in file/shell/web tools** out of the box — consider OpenAI Agents SDK, Dify, or CrewAI
- You want a **visual workflow builder** — consider Dify, LangGraph Studio
- You want **Python** and rapid prototyping — consider LangGraph, AG2, PydanticAI
- You need a **stable, slow-moving surface area** more than an evolving runtime platform
- You need **LLM-managed memory** (agent decides what to remember) — consider Letta

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

### Persist conversations

Swap storage backends without changing agent code:

| Backend | Use case |
|---|---|
| `InMemoryStore` | Tests and ephemeral demos |
| `FileStore` | Local development, single-server deployment |
| `PostgresStore` | Production with SQL queries and backups |

## Architecture

Awaken is split into three runtime layers. `awaken-contract` defines the shared contracts: agent specs, model/provider specs, tools, events, transport traits, and the typed state model. `awaken-runtime` resolves an `AgentSpec` into a `ResolvedAgent`, builds an `ExecutionEnv` from plugins, executes the phase loop, and manages active runs plus external control such as cancellation and HITL decisions. `awaken-server` exposes that same runtime through HTTP routes, SSE replay, mailbox-backed background execution, and protocol adapters for AI SDK v6, AG-UI, A2A, and MCP.

Around those layers sit storage and extensions. `awaken-stores` provides memory, file, and PostgreSQL backends for threads and runs. `awaken-ext-*` crates extend the runtime at phase and tool boundaries with permission checks, observability, MCP tool discovery, skills, reminders, generative UI, and deferred-tool loading.

### Repository Map

```text
awaken                   Facade crate with feature flags
├─ awaken-contract       Contracts: specs, tools, events, transport, state model
├─ awaken-runtime        Resolver, phase engine, loop runner, runtime control
├─ awaken-server         Routes, mailbox, SSE transport, protocol adapters
├─ awaken-stores         Memory, file, and PostgreSQL persistence
├─ awaken-tool-pattern   Glob/regex matching used by extensions
└─ awaken-ext-*          Optional runtime extensions
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

Additional workspace extension crates may exist without facade feature flags. Today that includes `awaken-ext-deferred-tools`.

> `awaken-ext-deferred-tools` is a standalone crate not included in the `full` feature. Add it as a direct dependency if you need lazy tool loading.

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
| [`docs/adr/`](./docs/adr/) | 22 Architecture Decision Records |
| [`DEVELOPMENT.md`](./DEVELOPMENT.md) | Build, test, and contribution guide |
| [`CONTRIBUTING.md`](./CONTRIBUTING.md) | Contributor guidelines |

> The English and Chinese book pages now share the same table of contents. Update both when APIs, routes, or configuration surfaces change.

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
