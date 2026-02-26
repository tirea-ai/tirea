"use client";

import { CSSProperties, ReactNode } from "react";

type StarterSplitScreenProps = {
  rootTestId: string;
  pageStyle: CSSProperties;
  chatHint: string;
  left: ReactNode;
  chat: ReactNode;
};

export function StarterSplitScreen({
  rootTestId,
  pageStyle,
  chatHint,
  left,
  chat,
}: StarterSplitScreenProps) {
  return (
    <main data-testid={rootTestId} className="relative isolate min-h-screen overflow-hidden text-white" style={pageStyle}>
      <div
        aria-hidden
        className="pointer-events-none absolute inset-0 -z-10 bg-[radial-gradient(500px_300px_at_5%_5%,rgba(255,255,255,0.3),transparent_70%),radial-gradient(600px_400px_at_95%_15%,rgba(236,252,203,0.22),transparent_72%)]"
      />
      <div className="grid h-screen grid-cols-1 lg:grid-cols-[minmax(0,1fr)_460px]">
        <section className="min-h-0 min-w-0 overflow-hidden">
          <div className="h-full overflow-y-auto px-4 py-5 lg:px-5">
            {left}
          </div>
        </section>
        <aside className="min-h-[66vh] border-t border-white/30 bg-white/10 p-2 backdrop-blur-md lg:min-h-0 lg:border-l lg:border-t-0">
          <div className="chat-theme flex h-full flex-col overflow-hidden rounded-2xl border border-slate-200 bg-white/95 shadow-[0_18px_36px_rgba(15,23,42,0.16)]">
            <header className="border-b border-slate-200 bg-gradient-to-b from-slate-50 to-slate-100 px-4 py-3">
              <h2 className="m-0 text-base font-semibold text-slate-900">tirea assistant</h2>
              <p className="mt-1 text-xs text-slate-600">{chatHint}</p>
            </header>
            <div className="starter-assistant-body min-h-0 flex-1 overflow-hidden">{chat}</div>
          </div>
        </aside>
      </div>
    </main>
  );
}
