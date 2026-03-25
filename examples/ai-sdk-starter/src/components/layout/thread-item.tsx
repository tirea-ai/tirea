import { useState, useRef, useEffect } from "react";
import type { ThreadSummary } from "@/hooks/use-threads";

function relativeTime(ts: number | null): string {
  if (!ts) return "";
  const diff = Date.now() - ts;
  if (diff < 60_000) return "just now";
  if (diff < 3_600_000) return `${Math.floor(diff / 60_000)}m ago`;
  if (diff < 86_400_000) return `${Math.floor(diff / 3_600_000)}h ago`;
  if (diff < 604_800_000) return `${Math.floor(diff / 86_400_000)}d ago`;
  return new Date(ts).toLocaleDateString();
}

type ThreadItemProps = {
  thread: ThreadSummary;
  isActive: boolean;
  onSelect: () => void;
  onRename: (title: string) => void;
  onDelete: () => void;
};

export function ThreadItem({
  thread,
  isActive,
  onSelect,
  onRename,
  onDelete,
}: ThreadItemProps) {
  const [menuOpen, setMenuOpen] = useState(false);
  const [renaming, setRenaming] = useState(false);
  const [renameValue, setRenameValue] = useState("");
  const inputRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    if (renaming) {
      inputRef.current?.focus();
      inputRef.current?.select();
    }
  }, [renaming]);

  const displayTitle = thread.title || `Chat ${thread.id.slice(0, 8)}`;

  const commitRename = () => {
    const trimmed = renameValue.trim();
    if (trimmed && trimmed !== thread.title) {
      onRename(trimmed);
    }
    setRenaming(false);
  };

  return (
    <div
      data-testid={`thread-item-${thread.id}`}
      className={`group relative flex cursor-pointer items-center gap-2 rounded-lg px-3 py-2 text-sm transition ${
        isActive
          ? "bg-slate-200 font-medium text-slate-900"
          : "text-slate-700 hover:bg-slate-100"
      }`}
      onClick={() => {
        if (!renaming) onSelect();
      }}
    >
      <div className="min-w-0 flex-1">
        {renaming ? (
          <input
            ref={inputRef}
            value={renameValue}
            onChange={(e) => setRenameValue(e.target.value)}
            onBlur={commitRename}
            onKeyDown={(e) => {
              if (e.key === "Enter") commitRename();
              if (e.key === "Escape") setRenaming(false);
            }}
            className="w-full rounded border border-slate-300 bg-white px-1.5 py-0.5 text-sm outline-none ring-cyan-300 focus:ring-1"
            onClick={(e) => e.stopPropagation()}
          />
        ) : (
          <>
            <div className="truncate">{displayTitle}</div>
            <div className="truncate text-xs text-slate-400">
              {relativeTime(thread.updated_at)}
            </div>
          </>
        )}
      </div>

      {!renaming && (
        <div className="relative shrink-0">
          <button
            type="button"
            onClick={(e) => {
              e.stopPropagation();
              setMenuOpen((v) => !v);
            }}
            className="invisible rounded p-0.5 text-slate-400 hover:bg-slate-200 hover:text-slate-600 group-hover:visible"
            aria-label="Thread options"
          >
            <svg width="16" height="16" viewBox="0 0 16 16" fill="currentColor">
              <circle cx="8" cy="3" r="1.5" />
              <circle cx="8" cy="8" r="1.5" />
              <circle cx="8" cy="13" r="1.5" />
            </svg>
          </button>

          {menuOpen && (
            <>
              <div
                className="fixed inset-0 z-10"
                onClick={(e) => {
                  e.stopPropagation();
                  setMenuOpen(false);
                }}
              />
              <div className="absolute right-0 top-6 z-20 w-32 rounded-lg border border-slate-200 bg-white py-1 shadow-lg">
                <button
                  type="button"
                  className="w-full px-3 py-1.5 text-left text-sm text-slate-700 hover:bg-slate-50"
                  onClick={(e) => {
                    e.stopPropagation();
                    setMenuOpen(false);
                    setRenameValue(thread.title || "");
                    setRenaming(true);
                  }}
                >
                  Rename
                </button>
                <button
                  type="button"
                  className="w-full px-3 py-1.5 text-left text-sm text-red-600 hover:bg-red-50"
                  onClick={(e) => {
                    e.stopPropagation();
                    setMenuOpen(false);
                    onDelete();
                  }}
                >
                  Delete
                </button>
              </div>
            </>
          )}
        </div>
      )}
    </div>
  );
}
