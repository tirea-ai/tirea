import { describe, expect, it } from "vitest";
import {
  parseHistoryPayload,
  parseInferenceMetrics,
  parseThreadSummariesPayload,
} from "./protocol";

describe("parseInferenceMetrics", () => {
  it("reads the ai-sdk inference payload shape", () => {
    const metrics = parseInferenceMetrics({
      model: "glm-4-flash-250414",
      durationMs: 822,
      usage: {
        prompt_tokens: 2590,
        completion_tokens: 3,
        total_tokens: 2593,
      },
    });

    expect(metrics).toEqual({
      model: "glm-4-flash-250414",
      durationMs: 822,
      usage: {
        prompt_tokens: 2590,
        completion_tokens: 3,
        total_tokens: 2593,
      },
    });
  });

  it("rejects metrics without durationMs", () => {
    expect(
      parseInferenceMetrics({
        model: "glm-4-flash-250414",
      }),
    ).toEqual({
      model: "glm-4-flash-250414",
      durationMs: 0,
      usage: undefined,
    });
  });
});

describe("parseHistoryPayload", () => {
  it("reads messages arrays", () => {
    const messages = parseHistoryPayload({
      messages: [{ id: "m1", role: "user", parts: [{ type: "text", text: "hi" }] }],
    });

    expect(messages).toHaveLength(1);
    expect(messages[0]?.id).toBe("m1");
  });

  it("ignores nonstandard payload shapes", () => {
    const messages = parseHistoryPayload({
      items: [{ id: "m2", role: "assistant", parts: [{ type: "text", text: "ok" }] }],
    });

    expect(messages).toEqual([]);
  });
});

describe("parseThreadSummariesPayload", () => {
  it("reads thread summaries from an items envelope", () => {
    const threads = parseThreadSummariesPayload({
      items: [
        {
          id: "thread-1",
          title: "First thread",
          updated_at: 123,
        },
      ],
    });

    expect(threads).toEqual([
      {
        id: "thread-1",
        title: "First thread",
        updated_at: 123,
        created_at: null,
        message_count: 0,
        agentId: null,
      },
    ]);
  });

  it("reads agent ownership from snake_case or camelCase fields", () => {
    const threads = parseThreadSummariesPayload({
      items: [
        {
          id: "thread-1",
          title: "A2UI thread",
          updated_at: 123,
          agent_id: "a2ui",
        },
        {
          id: "thread-2",
          title: "OpenUI thread",
          updated_at: 124,
          agentId: "openui",
        },
      ],
    });

    expect(threads).toEqual([
      {
        id: "thread-1",
        title: "A2UI thread",
        updated_at: 123,
        created_at: null,
        message_count: 0,
        agentId: "a2ui",
      },
      {
        id: "thread-2",
        title: "OpenUI thread",
        updated_at: 124,
        created_at: null,
        message_count: 0,
        agentId: "openui",
      },
    ]);
  });
});
