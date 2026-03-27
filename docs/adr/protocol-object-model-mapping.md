# Protocol Object Model Mapping

How external protocol concepts map to awaken's internal model.

## Core Principle

**Runs are internal state.** External consumers (frontends, SDKs) only see **threads**. A thread may have many runs over its lifetime, but the run_id is never exposed in API URLs or required by clients.

## Object Model

| awaken | AI SDK (Vercel) | AG-UI (CopilotKit) | A2A | ACP |
|--------|-----------------|---------------------|-----|-----|
| `thread_id` | `chatId` / `id` | `threadId` | `taskId` | `threadId` |
| `run` (internal) | — | — | — | `runId` (in ACK only) |
| `agent_id` | `agentId` | `agentId` | `agentId` | `agentId` |
| `Message` | `messages[]` | `messages[]` | `message.parts[]` | `message` |
| `AgentEvent` stream | UI Stream Protocol | AG-UI events | — (polling) | JSON-RPC notifications |

## AI SDK Mapping Details

| AI SDK concept | awaken mapping | Notes |
|----------------|----------------|-------|
| `chatId` | `thread_id` | AI SDK uses this as the session identifier. awaken generates a UUID if not provided. |
| `messages` | `Vec<Message>` | AI SDK sends `{role, content}` objects. Content can be string or array of parts (text, image_url). |
| `reconnectToStream(chatId)` | `GET /v1/ai-sdk/chat/{thread_id}/stream` | AI SDK calls `{api}/{chatId}/stream`. Returns 204 if no active stream. |
| `sendMessages(...)` | `POST /v1/ai-sdk/chat` | Creates a new run on the thread. |
| UI Stream events | `AiSdkEncoder` output | `TextStart`, `TextDelta`, `ToolInputStart`, `Finish`, etc. |

### SSE Resumption Flow

```
1. Client calls POST /v1/ai-sdk/chat with {messages, threadId}
2. Server creates run, registers replay buffer keyed by thread_id
3. Server streams SSE frames with `id: N` fields
4. Client disconnects (network, tab sleep, etc.)
5. AI SDK calls GET /v1/ai-sdk/chat/{threadId}/stream
6. Server returns 204 (no active stream) or replays buffer + chains live events
```

## AG-UI Mapping Details

| AG-UI concept | awaken mapping | Notes |
|---------------|----------------|-------|
| `threadId` | `thread_id` | Direct mapping. |
| `messages` | `Vec<Message>` | AG-UI sends `{role, content}` with content as array of `{type, text}` or media parts. |
| `runId` | — | AG-UI protocol includes `runId` in events, but clients don't use it for reconnection. |
| Run events | `AgUiEncoder` output | `RUN_STARTED`, `TEXT_MESSAGE_START`, `RUN_FINISHED`, etc. |

AG-UI does not have a built-in reconnection mechanism. Thread state recovery relies on `loadThreadSnapshot` which hydrates from persistent storage at the start of a new run.

## A2A Mapping Details

| A2A concept | awaken mapping | Notes |
|-------------|----------------|-------|
| `taskId` | `thread_id` | A2A maps task lifecycle to awaken thread lifecycle. |
| Task state | Latest run status | `GET /v1/a2a/tasks/:task_id` queries the latest run on that thread. |
| `message.parts[].text` | `Message` with `ContentBlock::Text` | A2A uses a flat text-parts model. |

A2A is fire-and-forget (no streaming). Status is polled via `GET /v1/a2a/tasks/:task_id`.

## ACP Mapping Details

| ACP concept | awaken mapping | Notes |
|-------------|----------------|-------|
| `threadId` | `thread_id` | Direct mapping. |
| `runId` | `run_id` | Returned in the initial ACK, but not required for subsequent operations. |
| `session/update` | JSON-RPC notification with `AcpEvent` | Streamed over stdio. |
| `session/update` (client→server) | Tool call decision | Client sends resume/cancel decisions. |

ACP uses stdio (not HTTP), so SSE resumption does not apply.
