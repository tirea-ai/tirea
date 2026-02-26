"use client";

import { CSSProperties, useEffect, useState } from "react";
import { CopilotKit, useCoAgent, useCopilotAction } from "@copilotkit/react-core";
import { CopilotChat } from "@copilotkit/react-ui";
import { StarterNavTabs } from "@/components/starter-nav-tabs";
import { RecommendedActions } from "@/components/recommended-actions";
import {
  RECOMMENDED_ACTIONS,
  V1_RECOMMENDED_SUGGESTIONS,
} from "@/lib/recommended-actions";
import { StarterSplitScreen } from "@/components/starter-split-screen";

type DemoState = {
  todos: string[];
};

type Thread = {
  id: string;
  title: string;
};

const FALLBACK_THREAD_ID = "local-fallback";

function ThreadChat({
  threads,
  activeThreadId,
  onCreateThread,
  onSelectThread,
}: {
  threads: Thread[];
  activeThreadId: string;
  onCreateThread: () => void;
  onSelectThread: (id: string) => void;
}) {
  const [themeColor, setThemeColor] = useState("#0f766e");
  const [localTodo, setLocalTodo] = useState("Persist this todo to current thread");
  const { state, setState } = useCoAgent<DemoState>({
    name: "default",
    initialState: { todos: [] },
  });

  useCopilotAction({
    name: "addTodo",
    description: "Append a todo in current persisted thread state",
    parameters: [{ name: "text", type: "string", required: true }],
    handler: async ({ text }: { text: string }) => {
      setState({ ...state, todos: [...(state?.todos ?? []), text] });
    },
  });

  useCopilotAction({
    name: "setThemeColor",
    description: "Set page theme color",
    parameters: [{ name: "color", type: "string", required: true }],
    handler: async ({ color }: { color: string }) => setThemeColor(color),
  });

  const pageStyle = {
    backgroundColor: themeColor,
    ["--page-accent" as "--page-accent"]: themeColor,
  } as CSSProperties;

  const leftPane = (
    <div className="mx-auto max-w-[1040px]">
      <div className="inline-flex items-center gap-2 rounded-full border border-white/50 bg-slate-900/30 px-3 py-1 text-xs font-semibold tracking-wide text-slate-100">
        CopilotKit x tirea starter
      </div>
      <h1 data-testid="page-title" className="mt-3 text-4xl font-bold tracking-tight text-white md:text-5xl">
        with-tirea persisted threads
      </h1>
      <p className="mt-2 max-w-[760px] text-sm text-slate-100/95 md:text-base">
        Backend-persisted thread demo: switch thread, keep history and state.
      </p>
      <StarterNavTabs mode="threads" />
      <RecommendedActions title="Recommended Actions" actions={RECOMMENDED_ACTIONS} defaultStack="v1" />

      <div className="mt-4 grid gap-3 rounded-2xl border border-white/60 bg-white/80 p-5 text-slate-900 shadow-[0_20px_45px_rgba(15,23,42,0.14)] backdrop-blur">
        <section className="rounded-xl border border-slate-200 bg-white/75 p-3">
          <div className="flex flex-wrap items-start justify-between gap-3">
            <div>
              <div className="mb-2 inline-flex items-center rounded-full border border-blue-200 bg-blue-50 px-2 py-0.5 text-xs font-bold text-blue-700">
                Persisted Threads
              </div>
              <h2 className="text-xl font-semibold text-slate-900">Thread State</h2>
              <p data-testid="active-thread" className="mt-1 text-sm text-slate-700">
                Active thread: {activeThreadId}
              </p>
            </div>
            <button
              data-testid="thread-create"
              onClick={onCreateThread}
              className="min-w-32 rounded-lg border border-slate-300 bg-white px-3 py-2 text-sm font-semibold text-slate-800 transition hover:-translate-y-px hover:border-slate-400 hover:bg-slate-50"
            >
              + New Thread
            </button>
          </div>
          <p data-testid="thread-prompt" className="mt-2 text-sm text-slate-600">
            Prompt: "Add todo: verify persisted thread state."
          </p>
          <div data-testid="thread-list" className="mt-3 grid grid-cols-[repeat(auto-fill,minmax(170px,1fr))] gap-2">
            {threads.map((thread) => (
              <button
                key={thread.id}
                data-testid={`thread-item-${thread.id}`}
                onClick={() => onSelectThread(thread.id)}
                className={`rounded-lg border px-3 py-2 text-left transition ${
                  activeThreadId === thread.id
                    ? "border-cyan-700 bg-cyan-50 text-cyan-900"
                    : "border-slate-200 bg-white text-slate-900 hover:-translate-y-px hover:border-slate-300"
                }`}
              >
                <div className="text-sm font-semibold">{thread.title}</div>
                <div className="text-xs text-slate-500">{thread.id.slice(0, 12)}</div>
              </button>
            ))}
          </div>
          <p data-testid="todo-count" className="mt-3 text-sm text-slate-700">
            Count: {(state?.todos ?? []).length}
          </p>
          <ul data-testid="todo-list" className="mt-1 list-disc space-y-1 pl-5 text-sm text-slate-700">
            {(state?.todos ?? []).map((todo, index) => (
              <li key={`${todo}-${index}`}>{todo}</li>
            ))}
          </ul>
          <div className="mt-3 flex flex-wrap items-center gap-2">
            <input
              data-testid="local-todo-input"
              value={localTodo}
              onChange={(event) => setLocalTodo(event.target.value)}
              placeholder="Type a todo"
              className="min-w-[180px] flex-1 rounded-lg border border-slate-300 bg-white px-3 py-2 text-sm text-slate-900 outline-none ring-cyan-300 transition focus:ring-2"
            />
            <button
              data-testid="add-local-todo"
              className="rounded-lg border border-slate-300 bg-white px-3 py-2 text-sm font-semibold text-slate-800 transition hover:-translate-y-px hover:border-slate-400 hover:bg-slate-50"
              onClick={() => {
                const text = localTodo.trim();
                if (!text) return;
                setState({ ...state, todos: [...(state?.todos ?? []), text] });
                setLocalTodo("");
              }}
            >
              Add Todo
            </button>
            <button
              data-testid="clear-local-todos"
              className="rounded-lg border border-red-300 bg-red-50 px-3 py-2 text-sm font-semibold text-red-800 transition hover:-translate-y-px hover:border-red-400 hover:bg-red-100"
              onClick={() => setState({ todos: [] })}
            >
              Clear Todos
            </button>
          </div>
        </section>
      </div>
    </div>
  );

  return (
    <StarterSplitScreen
      rootTestId="persisted-root"
      pageStyle={pageStyle}
      chatHint="Try switching threads and asking to add todos."
      left={leftPane}
      chat={
        <CopilotChat
          className="starter-chat-unified h-full"
          suggestions={V1_RECOMMENDED_SUGGESTIONS}
          labels={{
            title: "tirea assistant",
            initial: "Try switching threads and asking to add todos.",
          }}
        />
      }
    />
  );
}

export default function PersistedThreadsPage() {
  const [threads, setThreads] = useState<Thread[]>([{ id: FALLBACK_THREAD_ID, title: "Thread 1" }]);
  const [activeThreadId, setActiveThreadId] = useState(() => threads[0].id);

  useEffect(() => {
    let cancelled = false;

    (async () => {
      try {
        const response = await fetch("/api/threads", { cache: "no-store" });
        if (!response.ok) return;

        const ids = (await response.json()) as string[];
        if (cancelled || !Array.isArray(ids) || ids.length === 0) return;

        const loaded = ids.map((id, index) => ({
          id,
          title: `Thread ${index + 1}`,
        }));
        setThreads(loaded);
        setActiveThreadId(loaded[0].id);
      } catch {
        // Backend may be offline while developing locally.
      }
    })();

    return () => {
      cancelled = true;
    };
  }, []);

  const createThread = () => {
    const id = crypto.randomUUID();
    const thread = { id, title: `Thread ${threads.length + 1}` };
    setThreads((prev) => [...prev, thread]);
    setActiveThreadId(id);
  };

  return (
    <CopilotKit runtimeUrl="/api/copilotkit" agent="default" threadId={activeThreadId}>
      <ThreadChat
        threads={threads}
        activeThreadId={activeThreadId}
        onCreateThread={createThread}
        onSelectThread={setActiveThreadId}
      />
    </CopilotKit>
  );
}
