# API Documentation

The full Rust API reference is generated from source code documentation using `cargo doc`.

## Viewing API Docs

Build and view the API documentation locally:

```bash
# Build all crate docs
cargo doc --workspace --no-deps --open

# Or use the unified build script
bash scripts/build-docs.sh
```

When using `scripts/build-docs.sh`, the API docs are available at `target/book/doc/`.

## Crate Index

| Crate | Description | API Docs |
|-------|-------------|----------|
| `tirea_state` | Core state management | [tirea_state](../doc/tirea_state/index.html) |
| `tirea_state_derive` | Derive macros | [tirea_state_derive](../doc/tirea_state_derive/index.html) |
| `tirea_contract` | Shared contracts | [tirea_contract](../doc/tirea_contract/index.html) |
| `tirea_agent_loop` | Agent loop runtime | [tirea_agent_loop](../doc/tirea_agent_loop/index.html) |
| `tirea_agentos` | Orchestration layer | [tirea_agentos](../doc/tirea_agentos/index.html) |
| `tirea_store_adapters` | Persistence adapters | [tirea_store_adapters](../doc/tirea_store_adapters/index.html) |
| `tirea_agentos_server` | Server gateway | [tirea_agentos_server](../doc/tirea_agentos_server/index.html) |
| `tirea` | Umbrella re-export crate | [tirea](../doc/tirea/index.html) |

## Key Entry Points

### tirea_state

- [`apply_patch`](../doc/tirea_state/fn.apply_patch.html) — Apply a single patch to state
- [`Patch`](../doc/tirea_state/struct.Patch.html) — Patch container
- [`Op`](../doc/tirea_state/enum.Op.html) — Operation types
- [`StateContext`](../doc/tirea_state/struct.StateContext.html) — Typed state access
- [`JsonWriter`](../doc/tirea_state/struct.JsonWriter.html) — Dynamic patch builder
- [`TireaError`](../doc/tirea_state/enum.TireaError.html) — Error types

### tirea_contract

- [`Thread`](../doc/tirea_contract/struct.Thread.html) — Persisted thread model
- [`RunContext`](../doc/tirea_contract/struct.RunContext.html) — Run-scoped execution context
- [`RunRequest`](../doc/tirea_contract/struct.RunRequest.html) — Unified protocol request
- [`Tool`](../doc/tirea_contract/trait.Tool.html) — Tool trait
- [`AgentStateStore`](../doc/tirea_contract/trait.AgentStateStore.html) — Persistence abstraction

### tirea_agent_loop

- [`run_loop`](../doc/tirea_agent_loop/runtime/loop_runner/fn.run_loop.html) — Non-stream loop execution
- [`run_loop_stream`](../doc/tirea_agent_loop/runtime/loop_runner/fn.run_loop_stream.html) — Streamed loop execution
- [`AgentConfig`](../doc/tirea_agent_loop/runtime/loop_runner/struct.AgentConfig.html) — Loop configuration

### tirea_agentos

- [`AgentOs`](../doc/tirea_agentos/orchestrator/struct.AgentOs.html) — Registry + run orchestration
- [`AgentOsBuilder`](../doc/tirea_agentos/orchestrator/struct.AgentOsBuilder.html) — Builder for wiring
- [`AgentDefinition`](../doc/tirea_agentos/orchestrator/struct.AgentDefinition.html) — Declarative agent config
