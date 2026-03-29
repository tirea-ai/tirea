import { afterEach, describe, expect, it, vi } from "vitest";
import { fetchHistory, patchThreadTitle } from "./api-client";

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
    expect(result.length).toBe(1);
    expect(result[0]?.id).toBe("m1");
  });

  it("returns an empty list for nonstandard history payloads", async () => {
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
    expect(result).toEqual([]);
  });
});

describe("patchThreadTitle", () => {
  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it("returns true when metadata patch succeeds", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn().mockResolvedValue({
        ok: true,
      }),
    );

    await expect(patchThreadTitle("thread-1", "hello")).resolves.toBe(true);
  });

  it("returns false when metadata patch fails", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn().mockResolvedValue({
        ok: false,
      }),
    );

    await expect(patchThreadTitle("thread-1", "hello")).resolves.toBe(false);
  });
});
