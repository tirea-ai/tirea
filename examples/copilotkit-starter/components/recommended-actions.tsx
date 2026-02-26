"use client";

import { useState } from "react";
import type { StarterAction } from "@/lib/recommended-actions";

export function RecommendedActions({
  title,
  actions,
  defaultStack = "v2",
}: {
  title: string;
  actions: StarterAction[];
  defaultStack?: "v2" | "v1";
}) {
  const [copiedId, setCopiedId] = useState<string | null>(null);
  const [activeStack, setActiveStack] = useState<"v2" | "v1">(defaultStack);

  const copyPrompt = async (id: string, prompt: string) => {
    try {
      await navigator.clipboard.writeText(prompt);
      setCopiedId(id);
      setTimeout(() => setCopiedId((current) => (current === id ? null : current)), 1200);
    } catch {
      setCopiedId(null);
    }
  };

  const visibleActions = actions.filter((action) => action.stack === activeStack);

  return (
    <section
      className="mt-4 rounded-2xl border border-white/60 bg-white/80 p-4 text-slate-900 shadow-[0_20px_45px_rgba(15,23,42,0.14)] backdrop-blur"
      data-testid="recommended-actions"
    >
      <h2 className="m-0 text-slate-900">{title}</h2>
      <p className="mt-2 text-sm text-slate-600">
        v1/v2 能力已聚合，点击可复制提示词直接粘贴到右侧会话框。
      </p>
      <div className="mt-3 flex items-center gap-2" role="tablist" aria-label="action stack tabs">
        <button
          type="button"
          role="tab"
          data-testid="recommended-tab-v2"
          aria-selected={activeStack === "v2"}
          className={`rounded-full px-3 py-1 text-xs font-semibold ${
            activeStack === "v2"
              ? "bg-cyan-700 text-white"
              : "border border-slate-300 bg-white text-slate-700"
          }`}
          onClick={() => setActiveStack("v2")}
        >
          v2 (Recommended)
        </button>
        <button
          type="button"
          role="tab"
          data-testid="recommended-tab-v1"
          aria-selected={activeStack === "v1"}
          className={`rounded-full px-3 py-1 text-xs font-semibold ${
            activeStack === "v1"
              ? "bg-slate-800 text-white"
              : "border border-slate-300 bg-white text-slate-700"
          }`}
          onClick={() => setActiveStack("v1")}
        >
          v1 (Compatibility)
        </button>
      </div>
      <div className="mt-3 grid gap-2">
        {visibleActions.map((action) => (
          <div
            key={action.id}
            className="rounded-xl border border-slate-200 bg-white/80 px-3 py-2"
            data-testid={`recommended-action-${action.id}`}
          >
            <div className="flex items-start justify-between gap-3">
              <div>
                <div className="flex items-center gap-2">
                  <span className="rounded-full bg-slate-100 px-2 py-0.5 text-[11px] font-semibold uppercase tracking-wider text-slate-600">
                    {action.stack}
                  </span>
                  <span className="text-xs text-cyan-800">{action.capability}</span>
                </div>
                <p className="mt-1 text-sm font-semibold text-slate-900">{action.title}</p>
                <p className="mt-1 text-sm text-slate-700">"{action.prompt}"</p>
              </div>
              <button
                type="button"
                className="shrink-0 rounded-lg border border-slate-300 bg-slate-50 px-2 py-1 text-xs font-semibold text-slate-700 hover:bg-slate-100"
                onClick={() => copyPrompt(action.id, action.prompt)}
              >
                {copiedId === action.id ? "Copied" : "Copy"}
              </button>
            </div>
          </div>
        ))}
      </div>
    </section>
  );
}
