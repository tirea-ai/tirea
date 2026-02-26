"use client";

import { useEffect, useState } from "react";

const STORAGE_KEY = "with-tirea.main-thread-id";
const FALLBACK_THREAD_ID = "with-tirea-main";

function generateThreadId(): string {
  if (typeof crypto !== "undefined" && typeof crypto.randomUUID === "function") {
    return `with-tirea-${crypto.randomUUID()}`;
  }
  return `${FALLBACK_THREAD_ID}-${Date.now()}`;
}

export function useMainThreadId() {
  const [threadId, setThreadId] = useState<string>(FALLBACK_THREAD_ID);

  useEffect(() => {
    if (typeof window === "undefined") return;
    const existing = window.localStorage.getItem(STORAGE_KEY);
    if (existing) {
      setThreadId(existing);
      return;
    }

    const nextId = generateThreadId();
    window.localStorage.setItem(STORAGE_KEY, nextId);
    setThreadId(nextId);
  }, []);

  return { threadId };
}
