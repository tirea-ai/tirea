# HTTP API

## Conventions

- Error response shape:

```json
{ "error": "<message>" }
```

- Stream responses use `text/event-stream`.
- Query `limit` is clamped to `1..=200`.

## Endpoints

Health:

- `GET /health`

Thread resources:

- `GET /v1/threads`
- `GET /v1/threads/:id`
- `GET /v1/threads/:id/messages`

Protocol run streams:

- `POST /v1/ag-ui/agents/:agent_id/runs`
- `POST /v1/ai-sdk/agents/:agent_id/runs`

Protocol-encoded message history:

- `GET /v1/ag-ui/threads/:id/messages`
- `GET /v1/ai-sdk/threads/:id/messages`

## Health

```bash
curl -i http://127.0.0.1:8080/health
```

Expected: `200 OK`.

## Threads

List threads:

```bash
curl 'http://127.0.0.1:8080/v1/threads?offset=0&limit=50&parent_thread_id=p-root'
```

Example response:

```json
{
  "items": ["thread-1", "thread-2"],
  "total": 2,
  "has_more": false
}
```

Get one thread:

```bash
curl 'http://127.0.0.1:8080/v1/threads/thread-1'
```

Not found:

```json
{ "error": "thread not found: thread-1" }
```

## Messages

Raw messages:

```bash
curl 'http://127.0.0.1:8080/v1/threads/thread-1/messages?after=10&limit=20&order=asc&visibility=internal&run_id=run-1'
```

Supported query params:

- `after`, `before`: cursor boundaries
- `limit`: page size (`1..=200`)
- `order`: `asc` or `desc`
- `visibility`: `internal`, `none`, or omitted (default user-visible set)
- `run_id`: filter by run id

Example response:

```json
{
  "messages": [
    {
      "cursor": 11,
      "id": "msg_11",
      "role": "user",
      "content": "hello"
    }
  ],
  "has_more": false,
  "next_cursor": 11,
  "prev_cursor": 11
}
```

Protocol-encoded messages use dedicated routes:

```bash
curl 'http://127.0.0.1:8080/v1/ag-ui/threads/thread-1/messages'
curl 'http://127.0.0.1:8080/v1/ai-sdk/threads/thread-1/messages'
```

## Run Streams

AI SDK v6 stream:

```bash
curl -N \
  -H 'content-type: application/json' \
  -d '{"sessionId":"thread-1","input":"hello","runId":"run-1"}' \
  http://127.0.0.1:8080/v1/ai-sdk/agents/assistant/runs
```

AG-UI stream:

```bash
curl -N \
  -H 'content-type: application/json' \
  -d '{"threadId":"thread-1","runId":"run-1","messages":[{"role":"user","content":"hello"}],"tools":[]}' \
  http://127.0.0.1:8080/v1/ag-ui/agents/assistant/runs
```

AI SDK response specifics:

- Header `x-vercel-ai-ui-message-stream: v1`
- Header `x-tirea-ai-sdk-version: v6`
- Stream trailer `data: [DONE]`

## Validation Errors

AI SDK `POST /v1/ai-sdk/agents/:agent_id/runs`:

- empty `sessionId` -> `400` (`bad request: sessionId cannot be empty`)
- empty `input` -> `400` (`bad request: input cannot be empty`)

AG-UI `POST /v1/ag-ui/agents/:agent_id/runs`:

- empty `threadId` -> `400`
- empty `runId` -> `400`

Shared:

- unknown `agent_id` -> `404` (`agent not found: <id>`)
