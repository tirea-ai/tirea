const BACKEND_URL =
  process.env.BACKEND_URL ?? "http://localhost:8080";

// Generate a unique session ID per browser session (stable across messages).
let sessionId: string | null = null;
function getSessionId(): string {
  if (!sessionId) {
    sessionId = `ai-sdk-${crypto.randomUUID()}`;
  }
  return sessionId;
}

export async function POST(req: Request) {
  const { messages } = await req.json();

  // Extract the last user message as input for our server
  const lastUserMsg = [...messages]
    .reverse()
    .find((m: { role: string }) => m.role === "user");

  if (!lastUserMsg) {
    return new Response("No user message found", { status: 400 });
  }

  const upstream = await fetch(
    `${BACKEND_URL}/v1/agents/default/runs/ai-sdk/sse`,
    {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({
        sessionId: getSessionId(),
        input: lastUserMsg.content,
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
    },
  });
}
