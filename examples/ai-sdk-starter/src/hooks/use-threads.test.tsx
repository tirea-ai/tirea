import { act, renderHook, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { useThreads } from "./use-threads";

vi.mock("@/lib/api-client", () => ({
  fetchThreadSummaries: vi.fn(),
  patchThreadTitle: vi.fn().mockResolvedValue(true),
  deleteThread: vi.fn().mockResolvedValue(undefined),
}));

const {
  fetchThreadSummaries,
} = await import("@/lib/api-client");

describe("useThreads", () => {
  afterEach(() => {
    vi.clearAllMocks();
  });

  it("does not reselect the first fetched thread after startNewChat", async () => {
    let resolveFetch: ((value: unknown) => void) | undefined;
    vi.mocked(fetchThreadSummaries).mockImplementation(
      () =>
        new Promise((resolve) => {
          resolveFetch = resolve;
        }) as Promise<Awaited<ReturnType<typeof fetchThreadSummaries>>>,
    );

    const { result } = renderHook(() => useThreads("a2ui"));

    act(() => {
      result.current.startNewChat();
    });

    resolveFetch?.([
      {
        id: "thread-1",
        title: "Existing thread",
        updated_at: 100,
        created_at: 100,
        message_count: 1,
        agentId: "a2ui",
      },
    ]);

    await waitFor(() => {
      expect(result.current.loaded).toBe(true);
    });

    expect(result.current.activeThreadId).toBeNull();
  });

  it("keeps an optimistic locally created thread when the initial fetch returns empty", async () => {
    let resolveFetch: ((value: unknown) => void) | undefined;
    vi.mocked(fetchThreadSummaries).mockImplementation(
      () =>
        new Promise((resolve) => {
          resolveFetch = resolve;
        }) as Promise<Awaited<ReturnType<typeof fetchThreadSummaries>>>,
    );

    const { result } = renderHook(() => useThreads("a2ui"));

    let createdThreadId = "";
    act(() => {
      createdThreadId = result.current.createThread();
    });

    resolveFetch?.([]);

    await waitFor(() => {
      expect(result.current.loaded).toBe(true);
    });

    expect(result.current.threads).toEqual([
      expect.objectContaining({ id: createdThreadId, agentId: "a2ui" }),
    ]);
    expect(result.current.activeThreadId).toBe(createdThreadId);
  });

  it("filters thread summaries to the active agent", async () => {
    vi.mocked(fetchThreadSummaries).mockResolvedValue([
      {
        id: "thread-a2ui",
        title: "A2UI thread",
        updated_at: 200,
        created_at: 200,
        message_count: 1,
        agentId: "a2ui",
      },
      {
        id: "thread-openui",
        title: "OpenUI thread",
        updated_at: 100,
        created_at: 100,
        message_count: 1,
        agentId: "openui",
      },
    ]);

    const { result } = renderHook(() => useThreads("a2ui"));

    await waitFor(() => {
      expect(result.current.loaded).toBe(true);
    });

    expect(result.current.threads).toEqual([
      expect.objectContaining({ id: "thread-a2ui" }),
    ]);
    expect(result.current.activeThreadId).toBe("thread-a2ui");
  });
});
