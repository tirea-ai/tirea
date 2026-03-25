import { useState, useEffect, useCallback } from "react";
import {
  fetchThreadSummaries,
  patchThreadTitle,
  deleteThread as apiDeleteThread,
  type ThreadSummary,
} from "@/lib/api-client";

export type { ThreadSummary };

export function useThreads() {
  const [threads, setThreads] = useState<ThreadSummary[]>([]);
  const [activeThreadId, setActiveThreadId] = useState<string | null>(null);
  const [loaded, setLoaded] = useState(false);

  const refreshThreadList = useCallback(async () => {
    const summaries = await fetchThreadSummaries();
    setThreads(summaries);
    return summaries;
  }, []);

  useEffect(() => {
    let cancelled = false;

    fetchThreadSummaries().then((summaries) => {
      if (cancelled) return;
      setThreads(summaries);
      if (summaries.length > 0) {
        setActiveThreadId(summaries[0].id);
      }
      setLoaded(true);
    });

    return () => {
      cancelled = true;
    };
  }, []);

  const startNewChat = useCallback(() => {
    setActiveThreadId(null);
  }, []);

  const createThread = useCallback(() => {
    const id = crypto.randomUUID();
    const thread: ThreadSummary = {
      id,
      title: null,
      updated_at: Date.now(),
      created_at: Date.now(),
      message_count: 0,
    };
    setThreads((prev) => [thread, ...prev]);
    setActiveThreadId(id);
    return id;
  }, []);

  const renameThread = useCallback(
    async (id: string, title: string) => {
      setThreads((prev) =>
        prev.map((t) => (t.id === id ? { ...t, title } : t)),
      );
      await patchThreadTitle(id, title);
    },
    [],
  );

  const removeThread = useCallback(
    async (id: string) => {
      setThreads((prev) => {
        const next = prev.filter((t) => t.id !== id);
        // If we removed the active thread, switch to next available
        if (activeThreadId === id) {
          setActiveThreadId(next.length > 0 ? next[0].id : null);
        }
        return next;
      });
      await apiDeleteThread(id);
    },
    [activeThreadId],
  );

  const autoTitle = useCallback(
    async (threadId: string, firstMessage: string) => {
      const title = firstMessage.slice(0, 50).trim() || "New Chat";
      setThreads((prev) =>
        prev.map((t) => (t.id === threadId ? { ...t, title } : t)),
      );
      await patchThreadTitle(threadId, title);
    },
    [],
  );

  return {
    threads,
    activeThreadId,
    setActiveThreadId,
    startNewChat,
    createThread,
    renameThread,
    removeThread,
    autoTitle,
    refreshThreadList,
    loaded,
  };
}
