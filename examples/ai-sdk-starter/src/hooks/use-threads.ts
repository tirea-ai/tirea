import { useState, useEffect, useCallback, useMemo, useRef } from "react";
import {
  fetchThreadSummaries,
  patchThreadTitle,
  deleteThread as apiDeleteThread,
} from "@/lib/api-client";
import type { ThreadSummary } from "@/lib/protocol";

export type { ThreadSummary };

function sortThreads(threads: ThreadSummary[]): ThreadSummary[] {
  return [...threads].sort((a, b) => {
    const aUpdated = a.updated_at ?? a.created_at ?? 0;
    const bUpdated = b.updated_at ?? b.created_at ?? 0;
    if (aUpdated !== bUpdated) return bUpdated - aUpdated;
    return a.id.localeCompare(b.id);
  });
}

function mergeThreads(
  fetched: ThreadSummary[],
  existing: ThreadSummary[],
): ThreadSummary[] {
  const merged = new Map(existing.map((thread) => [thread.id, thread]));

  for (const thread of fetched) {
    const current = merged.get(thread.id);
    merged.set(thread.id, current ? { ...current, ...thread } : thread);
  }

  return sortThreads([...merged.values()]);
}

function filterThreadsByAgent(
  threads: ThreadSummary[],
  agentId: string,
): ThreadSummary[] {
  return threads.filter((thread) => thread.agentId === agentId);
}

export function useThreads(agentId: string) {
  const [allThreads, setAllThreads] = useState<ThreadSummary[]>([]);
  const [activeThreadId, setActiveThreadIdState] = useState<string | null>(null);
  const [loaded, setLoaded] = useState(false);
  const hasUserSelectedThread = useRef(false);

  const setActiveThreadId = useCallback((id: string | null) => {
    hasUserSelectedThread.current = true;
    setActiveThreadIdState(id);
  }, []);

  const refreshThreadList = useCallback(async () => {
    const summaries = await fetchThreadSummaries();
    let mergedThreads: ThreadSummary[] = [];
    setAllThreads((prev) => {
      mergedThreads = mergeThreads(summaries, prev);
      return mergedThreads;
    });
    const visibleThreads = filterThreadsByAgent(mergedThreads, agentId);
    if (
      activeThreadId != null &&
      visibleThreads.length > 0 &&
      !visibleThreads.some((thread) => thread.id === activeThreadId)
    ) {
      setActiveThreadIdState(visibleThreads[0].id);
    }
    if (activeThreadId != null && visibleThreads.length === 0) {
      setActiveThreadIdState(null);
    }
    return visibleThreads;
  }, [activeThreadId, agentId]);

  useEffect(() => {
    let cancelled = false;

    fetchThreadSummaries().then((summaries) => {
      if (cancelled) return;
      let mergedThreads: ThreadSummary[] = [];
      setAllThreads((prev) => {
        mergedThreads = mergeThreads(summaries, prev);
        return mergedThreads;
      });
      const visibleThreads = filterThreadsByAgent(mergedThreads, agentId);
      if (!hasUserSelectedThread.current && visibleThreads.length > 0) {
        setActiveThreadIdState(visibleThreads[0].id);
      }
      setLoaded(true);
    });

    return () => {
      cancelled = true;
    };
  }, [agentId]);

  const startNewChat = useCallback(() => {
    setActiveThreadId(null);
  }, [setActiveThreadId]);

  const createThread = useCallback(() => {
    const id = crypto.randomUUID();
    const thread: ThreadSummary = {
      id,
      title: null,
      updated_at: Date.now(),
      created_at: Date.now(),
      message_count: 0,
      agentId,
    };
    setAllThreads((prev) => [thread, ...prev]);
    setActiveThreadId(id);
    return id;
  }, [agentId, setActiveThreadId]);

  const renameThread = useCallback(
    async (id: string, title: string) => {
      setAllThreads((prev) =>
        sortThreads(prev.map((t) => (t.id === id ? { ...t, title } : t))),
      );
      await patchThreadTitle(id, title);
    },
    [],
  );

  const removeThread = useCallback(
    async (id: string) => {
      setAllThreads((prev) => {
        const next = prev.filter((t) => t.id !== id);
        const visibleThreads = filterThreadsByAgent(next, agentId);
        if (activeThreadId === id) {
          setActiveThreadId(visibleThreads.length > 0 ? visibleThreads[0].id : null);
        }
        return next;
      });
      await apiDeleteThread(id);
    },
    [activeThreadId, agentId, setActiveThreadId],
  );

  const threads = useMemo(
    () => filterThreadsByAgent(allThreads, agentId),
    [agentId, allThreads],
  );

  return {
    threads,
    activeThreadId,
    setActiveThreadId,
    startNewChat,
    createThread,
    renameThread,
    removeThread,
    refreshThreadList,
    loaded,
  };
}
