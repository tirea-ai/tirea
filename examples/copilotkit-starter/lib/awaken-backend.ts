import type { Message, State } from "@ag-ui/client";
import { randomUUID } from "crypto";

export type ThreadSnapshot = {
  state: State;
  messages: Message[];
};

const ALLOWED_ROLES = new Set([
  "developer",
  "system",
  "assistant",
  "user",
  "tool",
]);

function backendUrl(): string {
  return process.env.BACKEND_URL ?? "http://localhost:38080";
}

function listThreadsUrl(): string {
  return `${backendUrl()}/v1/threads?offset=0&limit=200`;
}

function threadUrl(threadId: string): string {
  return `${backendUrl()}/v1/threads/${encodeURIComponent(threadId)}`;
}

function threadMessagesUrl(threadId: string): string {
  return `${backendUrl()}/v1/ag-ui/threads/${encodeURIComponent(
    threadId,
  )}/messages?order=asc&limit=200&visibility=none`;
}

function normalizeMessage(input: unknown): Message | null {
  if (!input || typeof input !== "object") return null;
  const raw = input as Record<string, unknown>;

  const role = typeof raw.role === "string" ? raw.role.toLowerCase() : "";
  if (!ALLOWED_ROLES.has(role)) return null;

  const id =
    typeof raw.id === "string" && raw.id.trim().length > 0
      ? raw.id
      : `msg_${randomUUID()}`;

  const out: Record<string, unknown> = { id, role };

  if (typeof raw.content === "string") {
    out.content = raw.content;
  } else if (Array.isArray(raw.content)) {
    out.content = raw.content;
  } else if (raw.content == null) {
    out.content = "";
  } else {
    out.content = JSON.stringify(raw.content);
  }

  const toolCallId =
    typeof raw.toolCallId === "string"
      ? raw.toolCallId
      : typeof raw.tool_call_id === "string"
        ? raw.tool_call_id
        : undefined;
  if (toolCallId) out.toolCallId = toolCallId;

  const toolCalls = Array.isArray(raw.toolCalls)
    ? raw.toolCalls
    : Array.isArray(raw.tool_calls)
      ? raw.tool_calls
      : undefined;
  if (toolCalls) out.toolCalls = toolCalls;

  return out as Message;
}

export async function listThreadIdsFromBackend(): Promise<string[]> {
  const response = await fetch(listThreadsUrl(), { cache: "no-store" });
  if (!response.ok) {
    throw new Error(`list threads failed (${response.status})`);
  }

  const payload = (await response.json()) as { items?: unknown };
  if (!Array.isArray(payload.items)) return [];
  return payload.items.filter((value): value is string => typeof value === "string");
}

export async function loadThreadSnapshotFromBackend(
  threadId: string,
): Promise<ThreadSnapshot | null> {
  const threadResponse = await fetch(threadUrl(threadId), { cache: "no-store" });
  if (threadResponse.status === 404) return null;
  if (!threadResponse.ok) {
    throw new Error(`load thread failed (${threadResponse.status})`);
  }

  const threadPayload = (await threadResponse.json()) as { state?: unknown };
  const state = (threadPayload.state ?? {}) as State;

  const messagesResponse = await fetch(threadMessagesUrl(threadId), {
    cache: "no-store",
  });
  let messages: Message[] = [];
  if (messagesResponse.ok) {
    const payload = (await messagesResponse.json()) as { messages?: unknown[] };
    messages = (payload.messages ?? [])
      .map(normalizeMessage)
      .filter((value): value is Message => value !== null);
  } else if (messagesResponse.status !== 404) {
    throw new Error(`load thread messages failed (${messagesResponse.status})`);
  }

  return { state, messages };
}
