# Expose HTTP SSE

Use this when your client consumes run events over HTTP streaming.

## Prerequisites

- `AgentOs` is already wired with tools and agents.
- You have a `ThreadReader` for thread query APIs.
- Network path allows long-lived HTTP responses.

## Endpoints

Run endpoints:

- `POST /v1/ag-ui/agents/:agent_id/runs`
- `POST /v1/ai-sdk/agents/:agent_id/runs`

Thread endpoints:

- `GET /v1/threads`
- `GET /v1/threads/:id`
- `GET /v1/threads/:id/messages`
- `GET /v1/ag-ui/threads/:id/messages`
- `GET /v1/ai-sdk/threads/:id/messages`

## Steps

1. Wire the router.

```rust,ignore
use std::sync::Arc;
use tirea_agentos_server::http::{self, AppState};

let app = http::router(AppState {
    os: Arc::new(agent_os),
    read_store,
});
```

2. Start server and call AI SDK stream.

```bash
curl -N \
  -H 'content-type: application/json' \
  -d '{"sessionId":"thread-1","input":"hello"}' \
  http://127.0.0.1:8080/v1/ai-sdk/agents/assistant/runs
```

3. Call AG-UI stream.

```bash
curl -N \
  -H 'content-type: application/json' \
  -d '{"threadId":"thread-2","runId":"run-1","messages":[{"role":"user","content":"hello"}],"tools":[]}' \
  http://127.0.0.1:8080/v1/ag-ui/agents/assistant/runs
```

4. Query persisted thread messages.

```bash
curl 'http://127.0.0.1:8080/v1/threads/thread-1/messages?limit=20'
```

## Verify

- Run endpoints return `200` with `content-type: text/event-stream`.
- AI SDK route sets `x-vercel-ai-ui-message-stream: v1` and stream ends with `data: [DONE]`.
- AG-UI stream includes run lifecycle events (for example `RUN_STARTED`, `RUN_FINISHED`).

## Common Errors

- `400` input validation (`sessionId`/`input`/`threadId`/`runId` empty).
- `404` unknown `agent_id`.
- Route mismatch: old legacy paths return `404`.

## Related

- [HTTP API](../reference/http-api.md)
- [Integrate AI SDK Frontend](./integrate-ai-sdk-frontend.md)
- [Integrate CopilotKit (AG-UI)](./integrate-copilotkit-ag-ui.md)
- [AG-UI Protocol](../reference/protocols/ag-ui.md)
- [AI SDK v6 Protocol](../reference/protocols/ai-sdk-v6.md)
