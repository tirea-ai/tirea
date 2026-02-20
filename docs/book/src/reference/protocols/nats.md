# NATS Protocol

Gateway subjects:

- `agentos.ag-ui.runs`
- `agentos.ai-sdk.runs`

## Request Payloads

AG-UI subject payload:

```json
{
  "agentId": "assistant",
  "request": {
    "threadId": "t1",
    "runId": "r1",
    "messages": [
      { "role": "user", "content": "hello" }
    ],
    "tools": []
  },
  "replySubject": "_INBOX.x"
}
```

AI SDK subject payload:

```json
{
  "agentId": "assistant",
  "sessionId": "t1",
  "input": "hello",
  "runId": "r1",
  "replySubject": "_INBOX.x"
}
```

## Reply Behavior

- Gateway publishes protocol-encoded run events to the reply subject.
- Reply subject can come from NATS message reply inbox or payload `replySubject`.
- If both are missing, request is rejected (`missing reply subject`).
- If request is invalid or agent resolve fails, gateway publishes one error event.

## Operational Notes

- NATS is transport only; run semantics still follow canonical `AgentEvent` lifecycle.
- AG-UI streams terminate with AG-UI run-finished events.
- AI SDK streams terminate with AI SDK finish semantics.
