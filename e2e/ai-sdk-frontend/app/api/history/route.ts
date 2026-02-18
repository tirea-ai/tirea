import { NextRequest, NextResponse } from "next/server";

const BACKEND_URL =
  process.env.BACKEND_URL ?? "http://localhost:8080";

export async function GET(req: NextRequest) {
  const sessionId = req.nextUrl.searchParams.get("sessionId");
  if (!sessionId) {
    return NextResponse.json({ messages: [] });
  }

  const upstream = await fetch(
    `${BACKEND_URL}/v1/ai-sdk/threads/${encodeURIComponent(sessionId)}/messages?limit=200`,
  );

  if (!upstream.ok) {
    // Thread not found or other error — return empty history.
    return NextResponse.json({ messages: [] });
  }

  const data = await upstream.json();
  return NextResponse.json(data);
}
