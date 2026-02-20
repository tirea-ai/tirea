# AG-UI Protocol

Endpoint:

- `POST /v1/ag-ui/agents/:agent_id/runs`

## Request

Core fields (`RunAgentRequest`):

- `threadId` (required)
- `runId` (required)
- `messages`
- `tools`
- `context` (optional)
- `state` (optional)
- `parentRunId`, `model`, `systemPrompt`, `config` (optional)

Minimal example:

```json
{
  "threadId": "thread-1",
  "runId": "run-1",
  "messages": [
    { "role": "user", "content": "Plan my weekend" }
  ],
  "tools": []
}
```

## Response Transport

- `Content-Type: text/event-stream`
- SSE `data:` frames carry AG-UI protocol JSON events.
- Typical lifecycle markers include `RUN_STARTED` and `RUN_FINISHED`.

## Validation and Errors

- missing/empty `threadId` -> `400`
- missing/empty `runId` -> `400`
- unknown `agent_id` -> `404`

Error body shape:

```json
{ "error": "bad request: threadId cannot be empty" }
```

## Mapping

`AgUiInputAdapter` converts AG-UI request into internal `RunRequest`, then `AgUiProtocolEncoder` maps internal `AgentEvent` back to AG-UI events.
