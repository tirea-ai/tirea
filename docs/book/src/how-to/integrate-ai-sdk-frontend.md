# Integrate AI SDK Frontend

Use this when your web app uses `@ai-sdk/react` and you want to connect it to `tirea-agentos-server`.

## Prerequisites

- `tirea-agentos-server` is reachable (default `http://localhost:8080`).
- You are building a Next.js frontend.
- AI SDK endpoint is enabled:
  - `POST /v1/ai-sdk/agents/:agent_id/runs`
  - `GET /v1/ai-sdk/threads/:id/messages`

## Steps

1. Add dependencies in your frontend:

```bash
npm install @ai-sdk/react ai
```

2. Create `app/api/chat/route.ts` as an SSE pass-through.

This route maps AI SDK UI messages to the backend request format (`sessionId`, `input`, `runId`):

```ts,ignore
const BACKEND_URL = process.env.BACKEND_URL ?? "http://localhost:8080";
const AGENT_ID = process.env.AGENT_ID ?? "default";

export async function POST(req: Request) {
  const body = await req.json();
  const { messages, agentId } = body;
  const agent = agentId || AGENT_ID;
  const sessionId = req.headers.get("x-session-id") || `ai-sdk-${crypto.randomUUID()}`;

  const lastUserMsg = [...messages].reverse().find((m: { role: string }) => m.role === "user");
  if (!lastUserMsg) return new Response("No user message found", { status: 400 });

  let input: string;
  if (typeof lastUserMsg.content === "string") {
    input = lastUserMsg.content;
  } else if (Array.isArray(lastUserMsg.parts)) {
    input = lastUserMsg.parts
      .filter((p: { type: string }) => p.type === "text")
      .map((p: { text: string }) => p.text)
      .join("");
  } else {
    return new Response("Could not extract message text", { status: 400 });
  }

  const upstream = await fetch(`${BACKEND_URL}/v1/ai-sdk/agents/${agent}/runs`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ sessionId, input, runId: crypto.randomUUID() }),
  });

  if (!upstream.ok) return new Response(await upstream.text(), { status: upstream.status });
  if (!upstream.body) return new Response("No response body from backend", { status: 502 });

  return new Response(upstream.body, {
    headers: {
      "Content-Type": "text/event-stream",
      "Cache-Control": "no-cache",
      Connection: "keep-alive",
      "X-Vercel-AI-UI-Message-Stream": "v1",
      "X-Session-Id": sessionId,
    },
  });
}
```

3. Use `useChat` with `DefaultChatTransport` in your page.

```tsx,ignore
"use client";

import { useChat } from "@ai-sdk/react";
import { DefaultChatTransport } from "ai";
import { useMemo } from "react";

export default function ChatPage() {
  const sessionId = "ai-sdk-demo-session";
  const transport = useMemo(
    () => new DefaultChatTransport({ headers: { "x-session-id": sessionId } }),
    [sessionId],
  );

  const { messages, sendMessage, status } = useChat({ transport });
  // render messages and submit input via sendMessage({ text })
}
```

4. Optional: add `app/api/history/route.ts` that proxies
`GET /v1/ai-sdk/threads/:id/messages` to preload history.

## Verify

- `POST /api/chat` returns `text/event-stream`.
- Response includes `X-Vercel-AI-UI-Message-Stream: v1`.
- UI displays token streaming and final assistant output.
- Reload with same session id replays history (if history proxy is enabled).

## Common Errors

- `400` No user message found: route failed to extract latest user input from `messages`.
- `400` from backend for empty `input` or `sessionId`.
- Missing stream headers causes client not to parse AI SDK stream correctly.

## Related

- [AI SDK v6 Protocol](../reference/protocols/ai-sdk-v6.md)
- [HTTP API](../reference/http-api.md)
- `e2e/ai-sdk-frontend/README.md`
- `e2e/ai-sdk-frontend/app/api/chat/route.ts`
