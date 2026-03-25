# Awaken

**A modular AI agent runtime for Rust. Define agents, tools, and state with compile-time safety -- then serve them over multiple protocols from a single binary.**

Awaken provides the core building blocks for AI agent systems: a phase-based execution model, typed state with scoping and merge strategies, a plugin lifecycle system, and a multi-protocol server. Connect to any LLM provider via [genai](https://crates.io/crates/genai).

## Key features

- **Phase-based execution** -- agents run through gather/execute/commit phases with snapshot isolation and deterministic replay (ADR-0001)
- **Typed state engine** -- state keys with scoping (thread/run/tool_call), merge strategies (exclusive/commutative), and immutable snapshots (ADR-0002)
- **Plugin lifecycle** -- hooks into typed phases for permission, observability, context management, skills, MCP, and more (ADR-0005)
- **Registry and resolve** -- serializable `AgentSpec` definitions resolved at runtime from registries; agents reference tools and plugins by ID (ADR-0010)
- **Multi-protocol server** -- serve AI SDK v6, AG-UI, A2A, and ACP from one binary
- **Extension system** -- self-contained `awaken-ext-*` crates with standardized layout (ADR-0013)
- **Storage backends** -- in-memory, file-based, and PostgreSQL stores
- **Mailbox architecture** -- durable job queuing with interrupt support for human-in-the-loop workflows (ADR-0019)

## Architecture

```
awaken (facade crate)
  awaken-contract     -- types, traits, state model, agent specs
  awaken-runtime      -- execution engine, plugin system, agent loop
  awaken-server       -- HTTP server, SSE, protocol adapters
  awaken-stores       -- memory, file, postgres storage backends
  awaken-tool-pattern -- glob/regex tool matching
  awaken-ext-*        -- extensions (permission, observability, mcp, skills, reminder, generative-ui)
```

The runtime executes agents through a phase loop: **gather** plugin hooks, **execute** scheduled actions, and **commit** state mutations. Plugins register tools, hooks, and state keys through `PluginRegistrar`. See `docs/adr/` for detailed architecture decisions.

## Feature flags

| Feature | Default | Description |
|---|:---:|---|
| `permission` | yes | Permission plugin with allow/deny/ask policies |
| `observability` | yes | OpenTelemetry-based LLM and tool call tracing |
| `mcp` | no | Model Context Protocol client integration |
| `skills` | no | Skill package discovery and activation |
| `server` | no | Multi-protocol HTTP server |
| `generative-ui` | no | Declarative UI components sent to frontends |
| `full` | no | All features enabled |

## Quick start

Add awaken to your project:

```toml
[dependencies]
awaken = "0.1"
```

Or with all features:

```toml
[dependencies]
awaken = { version = "0.1", features = ["full"] }
```

## Examples

Examples live in two locations: `crates/awaken/examples/` for core runtime usage, and `examples/` for full-stack server demos.

### Core runtime examples (`crates/awaken/examples/`)

| Example | Description |
|---|---|
| `live_test` | Basic LLM integration with a real provider |
| `multi_turn` | Multi-turn conversation with persistent thread storage |
| `tool_call_live` | Tool calling with a calculator tool |

Run with:

```bash
export OPENAI_API_KEY=<your-key>
cargo run --package awaken --example multi_turn
```

### Server examples (`examples/`)

| Example | Description |
|---|---|
| `starter_backend` | Minimal server setup |
| `ai_sdk_starter` | AI SDK v6 protocol server |
| `copilotkit_starter` | CopilotKit / AG-UI protocol server |
| `travel` | Travel planning with tool orchestration |
| `research` | Research agent with multi-step workflows |
| `generative_ui` | Server-driven UI components |

Run with:

```bash
cargo run --package awaken-examples --example starter_backend
```

## Documentation

- `docs/adr/` -- Architecture Decision Records (19 ADRs covering execution model, state, plugins, registry, extensions, mailbox, and more)
- `DEVELOPMENT.md` -- Local development setup, build, test, and contribution workflow
- `CLAUDE.md` -- AI assistant coding guidelines

## License

Dual-licensed under [MIT](./LICENSE-MIT) or [Apache-2.0](./LICENSE-APACHE).
