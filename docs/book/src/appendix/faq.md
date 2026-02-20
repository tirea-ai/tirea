# FAQ

## Why no mutable session object?

`Thread` + patch history gives deterministic replay and clearer persistence semantics.

## Should I call `run_loop` directly?

Prefer `AgentOs::run_stream` for production. It handles load/create, dedup, and persistence wiring.

## Is rustdoc enough as all reference docs?

No. Rust API reference is necessary but protocol, transport, and operations reference must still be documented in mdBook.
