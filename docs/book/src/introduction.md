# Introduction

**Tirea** is an immutable state-driven agent framework built in Rust. It combines typed JSON state management with an agent loop, providing full traceability of state changes, replay capability, and component isolation.

## Crate Overview

| Crate | Description |
|-------|-------------|
| `tirea-state` | Core library: typed state, JSON patches, apply, conflict detection |
| `tirea-state-derive` | Proc-macro for `#[derive(State)]` |
| `tirea-contract` | Shared contracts: thread/events/tools/plugins/runtime/storage/protocol |
| `tirea-agent-loop` | Loop runtime: inference, tool execution, stop policies, streaming |
| `tirea-agentos` | Orchestration layer: registry wiring, run preparation, persistence integration |
| `tirea-store-adapters` | Storage adapters: memory/file/postgres/nats-buffered |
| `tirea` | Umbrella crate that re-exports core modules |
| `tirea-agentos-server` | HTTP/SSE/NATS gateway server |

## Architecture

```text
┌─────────────────────────────────────────────────────┐
│  Application Layer                                    │
│  - Register tools, define agents, call run_stream    │
└─────────────────────────────────────────────────────┘
                         │
                         ▼
┌─────────────────────────────────────────────────────┐
│  AgentOs + Agent Loop                                │
│  - Prepare run, execute phases, emit events          │
└─────────────────────────────────────────────────────┘
                         │
                         ▼
┌─────────────────────────────────────────────────────┐
│  Thread + State Engine                               │
│  - Thread history, RunContext delta, apply_patch     │
└─────────────────────────────────────────────────────┘
```

## Core Principle

All state transitions follow a deterministic, pure-function model:

```text
State' = apply_patch(State, Patch)
```

- Same `(State, Patch)` always produces the same `State'`
- `apply_patch` never mutates its input
- Full history enables replay to any point in time

## What's in This Book

- **Tutorials** — Learn by building a first agent and first tool
- **How-to** — Task-focused implementation guides for integration and operations
- **Reference** — API, protocol, config, and schema lookup pages
- **Explanation** — Architecture and design rationale

For the full Rust API documentation, see the [API Reference](./reference/api.md).
