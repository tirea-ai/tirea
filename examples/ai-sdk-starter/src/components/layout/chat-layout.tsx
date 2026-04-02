import { type ReactNode, useState } from "react";
import type { ThreadSummary } from "@/hooks/use-threads";
import { ThreadItem } from "./thread-item";

type AgentOption = {
  readonly id: string;
  readonly title: string;
  readonly desc: string;
};

type ChatLayoutProps = {
  threads: ThreadSummary[];
  activeThreadId: string | null;
  onSelectThread: (id: string) => void;
  onNewChat: () => void;
  onRenameThread: (id: string, title: string) => void;
  onDeleteThread: (id: string) => void;
  agentOptions: readonly AgentOption[];
  activeAgentId: string;
  onAgentChange: (id: string) => void;
  children: ReactNode;
};

export function ChatLayout({
  threads,
  activeThreadId,
  onSelectThread,
  onNewChat,
  onRenameThread,
  onDeleteThread,
  agentOptions,
  activeAgentId,
  onAgentChange,
  children,
}: ChatLayoutProps) {
  const [sidebarOpen, setSidebarOpen] = useState(false);

  return (
    <main className="flex h-screen bg-white" data-testid="playground-root">
      {/* Mobile overlay */}
      {sidebarOpen && (
        <div
          className="fixed inset-0 z-20 bg-black/30 md:hidden"
          onClick={() => setSidebarOpen(false)}
        />
      )}

      {/* Sidebar */}
      <aside
        className={`fixed inset-y-0 left-0 z-30 flex w-64 shrink-0 flex-col border-r border-slate-200 bg-slate-50 transition-transform md:static md:translate-x-0 ${
          sidebarOpen ? "translate-x-0" : "-translate-x-full"
        }`}
      >
        <div className="flex items-center gap-2 border-b border-slate-200 p-3">
          <button
            type="button"
            onClick={onNewChat}
            className="flex-1 rounded-lg border border-slate-300 bg-white px-3 py-2 text-sm font-semibold text-slate-800 transition hover:border-slate-400 hover:bg-slate-50"
            data-testid="thread-create"
          >
            + New Chat
          </button>
          <button
            type="button"
            onClick={() => setSidebarOpen(false)}
            className="rounded p-1.5 text-slate-400 hover:bg-slate-200 hover:text-slate-600 md:hidden"
            aria-label="Close sidebar"
          >
            <svg width="18" height="18" viewBox="0 0 18 18" fill="none" stroke="currentColor" strokeWidth="2">
              <path d="M4 4l10 10M14 4L4 14" />
            </svg>
          </button>
        </div>

        <div className="flex-1 overflow-y-auto px-2 py-2">
          {threads.length === 0 ? (
            <div className="px-2 py-4 text-center text-xs text-slate-400">
              No conversations yet
            </div>
          ) : (
            threads.map((t) => (
              <ThreadItem
                key={t.id}
                thread={t}
                isActive={activeThreadId === t.id}
                onSelect={() => {
                  onSelectThread(t.id);
                  setSidebarOpen(false);
                }}
                onRename={(title) => onRenameThread(t.id, title)}
                onDelete={() => onDeleteThread(t.id)}
              />
            ))
          )}
        </div>

        <div className="border-t border-slate-200 p-3">
          <label className="text-xs font-medium text-slate-500">Agent</label>
          <select
            value={activeAgentId}
            onChange={(e) => onAgentChange(e.target.value)}
            className="mt-1 w-full rounded-lg border border-slate-300 bg-white px-2 py-1.5 text-sm text-slate-800 outline-none"
            data-testid="agent-selector"
          >
            {agentOptions.map((a) => (
              <option key={a.id} value={a.id}>
                {a.title}
              </option>
            ))}
          </select>
        </div>
        <div className="border-t border-slate-200 p-3">
          <a
            href="/admin"
            className="block rounded-lg border border-slate-300 bg-slate-50 px-3 py-2 text-center text-sm text-slate-600 transition hover:bg-slate-100"
          >
            Admin Panel
          </a>
        </div>
      </aside>

      {/* Main content */}
      <div className="flex min-w-0 flex-1 flex-col">
        {/* Mobile header with hamburger */}
        <div className="flex items-center border-b border-slate-200 px-3 py-2 md:hidden">
          <button
            type="button"
            onClick={() => setSidebarOpen(true)}
            className="rounded p-1.5 text-slate-500 hover:bg-slate-100"
            aria-label="Open sidebar"
          >
            <svg width="20" height="20" viewBox="0 0 20 20" fill="currentColor">
              <rect x="2" y="4" width="16" height="2" rx="1" />
              <rect x="2" y="9" width="16" height="2" rx="1" />
              <rect x="2" y="14" width="16" height="2" rx="1" />
            </svg>
          </button>
          <span className="ml-2 text-sm font-medium text-slate-700">awaken assistant</span>
        </div>

        <div className="min-h-0 flex-1 overflow-hidden">
          {children}
        </div>
      </div>
    </main>
  );
}
