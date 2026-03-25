import { handleRequest } from "@/lib/copilotkit-app";

function isValidSingleRouteEnvelope(payload: unknown): boolean {
  if (!payload || typeof payload !== "object" || Array.isArray(payload)) {
    return false;
  }
  const method = (payload as Record<string, unknown>).method;
  return typeof method === "string" && method.length > 0;
}

export async function POST(request: Request): Promise<Response> {
  let raw = "";
  try {
    raw = await request.text();
  } catch {
    return Response.json(
      { error: "invalid_request", message: "Invalid request body" },
      { status: 400 },
    );
  }

  if (raw.trim().length === 0) {
    return Response.json(
      { error: "invalid_request", message: "Empty JSON payload" },
      { status: 400 },
    );
  }

  let parsed: unknown;
  try {
    parsed = JSON.parse(raw);
  } catch {
    return Response.json(
      { error: "invalid_request", message: "Invalid JSON payload" },
      { status: 400 },
    );
  }

  if (!isValidSingleRouteEnvelope(parsed)) {
    return Response.json(
      { error: "invalid_request", message: "Missing method field" },
      { status: 400 },
    );
  }

  const headers = new Headers(request.headers);
  if (!headers.has("content-type")) {
    headers.set("content-type", "application/json");
  }

  return handleRequest(
    new Request(request.url, {
      method: "POST",
      headers,
      body: raw,
      signal: request.signal,
    }),
  );
}
