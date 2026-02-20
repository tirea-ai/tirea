# AI SDK v6 Protocol

Endpoint:

- `POST /v1/ai-sdk/agents/:agent_id/runs`

## Request

Core fields (`AiSdkV6RunRequest`):

- `sessionId` (required) -> internal `thread_id`
- `input` (required) -> one user `Message`
- `runId` (optional)

Example:

```json
{
  "sessionId": "thread-1",
  "input": "Summarize the latest messages",
  "runId": "run-1"
}
```

## Response Transport

- `Content-Type: text/event-stream`
- Header `x-vercel-ai-ui-message-stream: v1`
- Header `x-tirea-ai-sdk-version: v6`
- Stream ends with `data: [DONE]`

Example event sequence:

```json
{ "type": "start", "messageId": "..." }
{ "type": "text-start", "id": "0" }
{ "type": "text-delta", "id": "0", "delta": "Hello" }
{ "type": "text-end", "id": "0" }
{ "type": "finish", "finishReason": "stop" }
```

## Validation and Errors

- empty `sessionId` -> `400` (`sessionId cannot be empty`)
- empty `input` -> `400` (`input cannot be empty`)
- unknown `agent_id` -> `404`

Error body shape:

```json
{ "error": "bad request: input cannot be empty" }
```

Implementation constant: `AI_SDK_VERSION = "v6"`.
