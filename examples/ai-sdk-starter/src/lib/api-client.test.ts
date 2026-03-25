import { afterEach, describe, expect, it, vi } from "vitest";
import type { UIMessage } from "@ai-sdk/react";
import { fetchHistory } from "./api-client";

describe("fetchHistory", () => {
  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it("reads history from messages", async () => {
    const payload = {
      messages: [{ id: "m1", role: "user", parts: [{ type: "text", text: "hi" }] }],
    };
    vi.stubGlobal(
      "fetch",
      vi.fn().mockResolvedValue({
        ok: true,
        json: async () => payload,
      }),
    );

    const result = await fetchHistory("thread-1");
    expect((result as UIMessage[]).length).toBe(1);
    expect(result[0]?.id).toBe("m1");
  });

  it("falls back to items when messages is missing", async () => {
    const payload = {
      items: [{ id: "m2", role: "assistant", parts: [{ type: "text", text: "ok" }] }],
    };
    vi.stubGlobal(
      "fetch",
      vi.fn().mockResolvedValue({
        ok: true,
        json: async () => payload,
      }),
    );

    const result = await fetchHistory("thread-2");
    expect((result as UIMessage[]).length).toBe(1);
    expect(result[0]?.id).toBe("m2");
  });
});

