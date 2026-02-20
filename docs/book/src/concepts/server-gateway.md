# Server Gateway

`tirea-agentos-server` provides HTTP and NATS interfaces for running agents as a service.

## HTTP Server (Axum)

The HTTP server exposes REST + SSE endpoints built on Axum:

### Endpoints

| Method | Path | Description |
|--------|------|-------------|
| POST | `/agents/:agent/run` | Run an agent (SSE stream) |
| POST | `/agents/:agent/sessions/:id/run` | Continue a session |
| GET | `/sessions` | List sessions |
| GET | `/sessions/:id` | Get session details |
| GET | `/sessions/:id/messages` | Get session messages (paginated) |
| DELETE | `/sessions/:id` | Delete a session |

### Protocol Support

The server supports two SSE encoding protocols:

- **AI SDK v6** — Compatible with Vercel AI SDK frontend (`AiSdkEncoder`)
- **AG-UI** — Compatible with CopilotKit frontend (`AgUiEncoder`)

The protocol is selected via request headers or query parameters.

## NATS Transport

For microservice deployments, `tirea-agentos-server` can also listen on NATS subjects, enabling decoupled agent invocation.

## Configuration

The server requires:

- An `AgentOs` instance (with agents, tools, models registered)
- A thread store implementation for thread persistence (for example `tirea_store_adapters::FileStore`)
- Network binding configuration (host, port)

```rust,ignore
use tireaos_server::http::AppState;

let state = AppState {
    os: Arc::new(agent_os),
    read_store: thread_store.clone(),
};
```
