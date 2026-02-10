import { createSSEToDataStreamTransform } from "@/lib/stream-adapter";

const BACKEND_URL =
  process.env.BACKEND_URL ?? "http://localhost:8080";

export async function POST(req: Request) {
  const { messages } = await req.json();

  // Extract the last user message as input for our server
  const lastUserMsg = [...messages]
    .reverse()
    .find((m: { role: string }) => m.role === "user");

  if (!lastUserMsg) {
    return new Response("No user message found", { status: 400 });
  }

  // Use a stable session ID derived from the conversation
  const sessionId = "ai-sdk-frontend";

  const upstream = await fetch(
    `${BACKEND_URL}/v1/agents/default/runs/ai-sdk/sse`,
    {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({
        sessionId,
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

  // Pipe the SSE stream through our protocol adapter
  const transformed = upstream.body.pipeThrough(
    createSSEToDataStreamTransform()
  );

  return new Response(transformed, {
    headers: {
      "Content-Type": "text/plain; charset=utf-8",
      "X-Vercel-AI-Data-Stream": "v1",
    },
  });
}
