"use client";

import Link from "next/link";

type StarterMode = "v1" | "v2" | "threads";

export function StarterNavTabs({ mode }: { mode: StarterMode }) {
  const tabClass = (active: boolean) =>
    active
      ? "rounded-full border border-cyan-700 bg-cyan-700 px-3 py-1.5 text-sm font-semibold text-cyan-50 shadow-[0_10px_24px_rgba(14,116,144,0.24)]"
      : "rounded-full border border-slate-300 bg-white px-3 py-1.5 text-sm font-semibold text-slate-800 transition hover:-translate-y-px hover:border-cyan-600 hover:bg-cyan-50 hover:shadow-[0_8px_20px_rgba(14,116,144,0.16)]";

  return (
    <nav
      className="mt-3 flex w-fit flex-wrap items-center gap-2 rounded-full border border-slate-200 bg-white/80 p-1 backdrop-blur"
      data-testid="starter-mode-tabs"
    >
      <Link
        href="/"
        data-testid="canvas-link"
        className={tabClass(mode === "v2")}
      >
        v2 (Recommended)
      </Link>
      <Link
        href="/v1"
        data-testid="base-starter-link"
        className={tabClass(mode === "v1")}
      >
        v1 (Compatibility)
      </Link>
      <Link
        href="/persisted-threads"
        data-testid="persisted-threads-link"
        className={tabClass(mode === "threads")}
      >
        Persisted Threads
      </Link>
    </nav>
  );
}
