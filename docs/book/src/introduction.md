# Introduction

**Tirea** is an immutable state-driven agent framework built in Rust. It combines typed JSON state management with an AI agent loop, providing full traceability of state changes, replay capability, and component isolation.

## Crate Overview

| Crate | Description |
|-------|-------------|
| `tirea-state` | Core library: typed state, JSON patches, apply, conflict detection |
| `tirea-state-derive` | Proc-macro for `#[derive(State)]` |
| `tirea` | Agent framework: tools, sessions, streaming, plugins |
| `tirea-agentos-server` | HTTP/SSE/NATS gateway server |

## Architecture

```text
┌─────────────────────────────────────────────────────┐
│  Application Layer                                   │
│  - Register tools, run agent loop, persist sessions  │
└─────────────────────────────────────────────────────┘
                         │
                         ▼
┌─────────────────────────────────────────────────────┐
│  Pure Functions                                      │
│  - build_request, StreamCollector, Session::with_*  │
└─────────────────────────────────────────────────────┘
                         │
                         ▼
┌─────────────────────────────────────────────────────┐
│  Tool Layer                                          │
│  - Tool trait, Context, automatic patch collection   │
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

- **Concepts** — Understand the core ideas: immutable state, typed access, the agent loop
- **Guides** — Step-by-step tutorials for common tasks
- **Reference** — Detailed information on error types, operations, and the API

For the full Rust API documentation, see the [API Reference](./reference/api.md).
