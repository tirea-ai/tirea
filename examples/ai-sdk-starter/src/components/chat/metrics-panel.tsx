import type { InferenceMetrics } from "@/hooks/use-chat-session";

export function MetricsPanel({ metrics }: { metrics: InferenceMetrics[] }) {
  if (metrics.length === 0) return null;

  const totalPrompt = metrics.reduce(
    (sum, m) => sum + (m.usage?.prompt_tokens ?? 0),
    0,
  );
  const totalCompletion = metrics.reduce(
    (sum, m) => sum + (m.usage?.completion_tokens ?? 0),
    0,
  );

  return (
    <div
      data-testid="metrics-panel"
      className="border-t border-slate-200 bg-blue-50/80 px-4 py-2 text-xs"
    >
      <div className="flex items-center justify-between">
        <strong className="text-slate-700">Token Usage</strong>
        <span className="text-slate-500 tabular-nums">
          total: {totalPrompt + totalCompletion} (in: {totalPrompt} / out: {totalCompletion})
        </span>
      </div>
      {metrics.map((m, i) => {
        const promptTokens = m.usage?.prompt_tokens ?? 0;
        const completionTokens = m.usage?.completion_tokens ?? 0;
        const hasUsage = promptTokens > 0 || completionTokens > 0;
        return (
          <div
            key={i}
            data-testid="metrics-entry"
            className="mt-0.5 flex items-center gap-2 text-slate-600"
          >
            <span className="font-medium">{m.model}</span>
            {hasUsage && (
              <span className="tabular-nums">
                in: {promptTokens} / out: {completionTokens}
              </span>
            )}
            <span className="text-slate-400">({m.durationMs}ms)</span>
          </div>
        );
      })}
    </div>
  );
}
