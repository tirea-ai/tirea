# Tirea

Tirea is an immutable state-driven agent framework in Rust.
It combines typed JSON state management, deterministic patch application, and an agent runtime/orchestration stack.

## Workspace crates

- `tirea-state`: typed state + JSON patch apply/conflict detection
- `tirea-state-derive`: `#[derive(State)]` proc macro
- `tirea-contract`: shared runtime/tool/protocol contracts
- `tirea-agent-loop`: agent execution loop
- `tirea-agentos`: orchestration and composition
- `tirea-store-adapters`: memory/file/postgres/nats-buffered stores
- `tirea-agentos-server`: HTTP/SSE/NATS gateway
- `tirea`: umbrella re-export crate

## Quick start

### Prerequisites

- Rust toolchain (see `rust-toolchain.toml`)
- One model provider key (`OPENAI_API_KEY` or `DEEPSEEK_API_KEY`)

### Build and test

```bash
cargo build --workspace
cargo test --workspace
```

### Run server

```bash
DEEPSEEK_API_KEY=<your-key> cargo run --package tirea-agentos-server -- \
  --http-addr 127.0.0.1:8080
```

## Documentation

- Book source: `docs/book/src/`
- Build docs locally:

```bash
bash scripts/build-docs.sh
```

Then open `target/book/index.html`.

## Project source

The canonical repository is:

- https://github.com/tirea-ai/tirea

## Security and contributing

- Security policy: [`SECURITY.md`](./SECURITY.md)
- Contributing guide: [`CONTRIBUTING.md`](./CONTRIBUTING.md)
- Code of conduct: [`CODE_OF_CONDUCT.md`](./CODE_OF_CONDUCT.md)

## License

Dual-licensed under:

- MIT ([`LICENSE-MIT`](./LICENSE-MIT))
- Apache-2.0 ([`LICENSE-APACHE`](./LICENSE-APACHE))
