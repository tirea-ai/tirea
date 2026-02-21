const BACKEND_URL =
  process.env.BACKEND_URL ?? "http://localhost:8080";
const AGENT_ID = process.env.AGENT_ID ?? "default";

export async function POST(req: Request) {
  const body = await req.json();
  const { messages, agentId } = body;
  const agent = agentId || AGENT_ID;

  // Session ID from client header (stable across reloads via localStorage).
  const sessionId =
    req.headers.get("x-session-id") || `ai-sdk-${crypto.randomUUID()}`;

  const upstream = await fetch(
    `${BACKEND_URL}/v1/ai-sdk/agents/${agent}/runs`,
    {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({
        id: sessionId,
        messages,
        runId: crypto.randomUUID(),
      }),
    }
  );

  if (!upstream.ok) {
    const text = await upstream.text();
    return new Response(text, { status: upstream.status });
  }

  if (!upstream.body) {
    return new Response("No response body from backend", { status: 502 });
  }

  // Pass through the SSE stream directly — backend already emits
  // AI SDK v6 UI Message Stream protocol events.
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
