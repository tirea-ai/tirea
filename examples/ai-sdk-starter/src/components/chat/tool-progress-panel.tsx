import type { ToolCallProgressNode } from "@/hooks/use-chat-session";

type ToolProgressPanelProps = {
  progressByNodeId: Record<string, ToolCallProgressNode>;
};

function statusColor(status: string) {
  switch (status) {
    case "pending":
      return "bg-slate-400";
    case "running":
      return "bg-blue-500";
    case "done":
      return "bg-emerald-500";
    case "failed":
      return "bg-red-500";
    case "cancelled":
      return "bg-amber-500";
    default:
      return "bg-slate-400";
  }
}

function statusBadge(status: string) {
  const colors: Record<string, string> = {
    pending: "border-slate-400 bg-slate-50 text-slate-600",
    running: "border-blue-400 bg-blue-50 text-blue-700",
    done: "border-emerald-400 bg-emerald-50 text-emerald-700",
    failed: "border-red-400 bg-red-50 text-red-700",
    cancelled: "border-amber-400 bg-amber-50 text-amber-700",
  };
  return colors[status] ?? colors.pending;
}

function ProgressBar({
  progress,
  status,
}: {
  progress?: number;
  status: string;
}) {
  const pct = progress !== undefined ? Math.min(Math.round(progress * 100), 100) : 0;
  const barColor = statusColor(status);
  const isIndeterminate = progress === undefined && status === "running";

  return (
    <div className="relative h-2.5 w-full overflow-hidden rounded-full bg-slate-200">
      {isIndeterminate ? (
        <div
          className={`absolute inset-y-0 left-0 w-1/3 rounded-full ${barColor} animate-[indeterminate_1.5s_ease-in-out_infinite]`}
        />
      ) : (
        <div
          className={`h-full rounded-full transition-all duration-300 ease-out ${barColor}`}
          style={{ width: `${pct}%` }}
        />
      )}
    </div>
  );
}

export function ToolProgressPanel({ progressByNodeId }: ToolProgressPanelProps) {
  const entries = Object.values(progressByNodeId).sort(
    (a, b) => (b.updated_at_ms ?? 0) - (a.updated_at_ms ?? 0),
  );
  if (entries.length === 0) return null;

  return (
    <div
      data-testid="tool-progress-panel"
      className="border-t border-slate-200 bg-gradient-to-b from-slate-50 to-white px-4 py-3"
    >
      <div className="mb-2 text-xs font-semibold text-slate-500 uppercase tracking-wide">
        Tool Progress
      </div>
      <div className="space-y-3">
        {entries.slice(0, 8).map((node) => {
          const pct =
            node.progress !== undefined
              ? Math.min(Math.round(node.progress * 100), 100)
              : undefined;

          return (
            <div key={node.node_id} className="space-y-1">
              <div className="flex items-center justify-between gap-2">
                <div className="flex items-center gap-2 min-w-0">
                  <span className="text-sm font-medium text-slate-800 truncate">
                    {node.tool_name ?? node.call_id ?? node.node_id}
                  </span>
                  <span
                    className={`inline-flex items-center rounded-full border px-2 py-0.5 text-[10px] font-semibold uppercase ${statusBadge(node.status)}`}
                  >
                    {node.status}
                  </span>
                </div>
                {pct !== undefined && (
                  <span className="text-xs font-mono font-semibold text-slate-600 tabular-nums">
                    {pct}%
                  </span>
                )}
              </div>
              <ProgressBar progress={node.progress} status={node.status} />
              {node.message && (
                <div className="text-xs text-slate-500 truncate">{node.message}</div>
              )}
              {node.loaded !== undefined && node.total !== undefined && (
                <div className="text-[10px] text-slate-400 tabular-nums">
                  {Math.round(node.loaded)} / {Math.round(node.total)}
                </div>
              )}
            </div>
          );
        })}
      </div>
      <style>{`
        @keyframes indeterminate {
          0% { transform: translateX(-100%); }
          100% { transform: translateX(400%); }
        }
      `}</style>
    </div>
  );
}
