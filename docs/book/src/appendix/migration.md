# Migration Notes

## Terminology Updates

- `Session` -> `Thread`
- `ThreadReader/ThreadWriter/ThreadStore` -> `AgentStateReader/AgentStateWriter/AgentStateStore`
- Session routes -> `/v1/threads` routes

## Runtime Surface Updates

- Prefer `AgentOs::run_stream(RunRequest)` for app-level integration.
- Use `RunContext` as run-scoped mutable workspace.
- Use protocol adapters (`AG-UI`, `AI SDK v6`) for transport-specific request/response mapping.
