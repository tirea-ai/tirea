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
| `tirea` | Agent framework | [tirea](../doc/tirea/index.html) |
| `tireaos_server` | Server gateway | [tireaos_server](../doc/tireaos_server/index.html) |

## Key Entry Points

### tirea_state

- [`apply_patch`](../doc/tirea_state/fn.apply_patch.html) — Apply a single patch to state
- [`Patch`](../doc/tirea_state/struct.Patch.html) — Patch container
- [`Op`](../doc/tirea_state/enum.Op.html) — Operation types
- [`Context`](../doc/tirea_state/struct.Context.html) — Typed state access
- [`JsonWriter`](../doc/tirea_state/struct.JsonWriter.html) — Dynamic patch builder
- [`TireaError`](../doc/tirea_state/enum.TireaError.html) — Error types

### tirea

- [`Tool`](../doc/tirea/trait.Tool.html) — Tool trait
- [`Session`](../doc/tirea/struct.Session.html) — Immutable session
- [`AgentOs`](../doc/tirea/struct.AgentOs.html) — Agent wiring
- [`run_loop`](../doc/tirea/fn.run_loop.html) — Blocking agent loop
- [`run_loop_stream`](../doc/tirea/fn.run_loop_stream.html) — Streaming agent loop
- [`AgentPlugin`](../doc/tirea/trait.AgentPlugin.html) — Plugin trait
