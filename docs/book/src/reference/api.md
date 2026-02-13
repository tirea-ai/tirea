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
| `carve_state` | Core state management | [carve_state](../doc/carve_state/index.html) |
| `carve_state_derive` | Derive macros | [carve_state_derive](../doc/carve_state_derive/index.html) |
| `carve_agent` | Agent framework | [carve_agent](../doc/carve_agent/index.html) |
| `carve_agentos_server` | Server gateway | [carve_agentos_server](../doc/carve_agentos_server/index.html) |

## Key Entry Points

### carve_state

- [`apply_patch`](../doc/carve_state/fn.apply_patch.html) — Apply a single patch to state
- [`Patch`](../doc/carve_state/struct.Patch.html) — Patch container
- [`Op`](../doc/carve_state/enum.Op.html) — Operation types
- [`Context`](../doc/carve_state/struct.Context.html) — Typed state access
- [`JsonWriter`](../doc/carve_state/struct.JsonWriter.html) — Dynamic patch builder
- [`CarveError`](../doc/carve_state/enum.CarveError.html) — Error types

### carve_agent

- [`Tool`](../doc/carve_agent/trait.Tool.html) — Tool trait
- [`Session`](../doc/carve_agent/struct.Session.html) — Immutable session
- [`AgentOs`](../doc/carve_agent/struct.AgentOs.html) — Agent wiring
- [`run_loop`](../doc/carve_agent/fn.run_loop.html) — Blocking agent loop
- [`run_loop_stream`](../doc/carve_agent/fn.run_loop_stream.html) — Streaming agent loop
- [`AgentPlugin`](../doc/carve_agent/trait.AgentPlugin.html) — Plugin trait
