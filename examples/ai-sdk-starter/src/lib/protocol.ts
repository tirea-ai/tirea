import type { UIMessage } from "@ai-sdk/react";

export interface InferenceMetrics {
  model: string;
  usage?: {
    prompt_tokens?: number;
    completion_tokens?: number;
    total_tokens?: number;
  };
  durationMs: number;
}

export type ThreadSummary = {
  id: string;
  title: string | null;
  updated_at: number | null;
  created_at: number | null;
  message_count: number;
  agentId: string | null;
};

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null;
}

function asNumber(value: unknown): number | undefined {
  return typeof value === "number" && Number.isFinite(value) ? value : undefined;
}

function asString(value: unknown): string | undefined {
  return typeof value === "string" && value.length > 0 ? value : undefined;
}

export function parseInferenceMetrics(data: unknown): InferenceMetrics | null {
  if (!isRecord(data)) return null;

  const model = asString(data.model);
  if (!model) return null;

  const usage = isRecord(data.usage)
    ? {
        prompt_tokens: asNumber(data.usage.prompt_tokens),
        completion_tokens: asNumber(data.usage.completion_tokens),
        total_tokens: asNumber(data.usage.total_tokens),
      }
    : undefined;

  return {
    model,
    usage,
    durationMs: asNumber(data.durationMs) ?? 0,
  };
}

export function parseHistoryPayload(payload: unknown): UIMessage[] {
  return isRecord(payload) && Array.isArray(payload.messages)
    ? (payload.messages as UIMessage[])
    : [];
}

export function parseThreadSummariesPayload(payload: unknown): ThreadSummary[] {
  const items = isRecord(payload) && Array.isArray(payload.items)
    ? payload.items
    : [];

  return items
    .filter(isRecord)
    .map((item) => {
      const id = asString(item.id);
      if (!id) return null;

      return {
        id,
        title: typeof item.title === "string" ? item.title : null,
        updated_at: asNumber(item.updated_at) ?? null,
        created_at: asNumber(item.created_at) ?? null,
        message_count: asNumber(item.message_count) ?? 0,
        agentId:
          asString(item.agentId) ??
          asString(item.agent_id) ??
          null,
      } satisfies ThreadSummary;
    })
    .filter((item): item is ThreadSummary => item !== null);
}
