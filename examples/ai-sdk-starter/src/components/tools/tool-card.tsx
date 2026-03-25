import { useState } from "react";

type ToolCardProps = {
  name: string;
  state: string;
  input?: unknown;
  output?: unknown;
  errorText?: string;
};

export function ToolCard({ name, state, input, output, errorText }: ToolCardProps) {
  const [showInput, setShowInput] = useState(false);
  const [showOutput, setShowOutput] = useState(false);

  return (
    <div
      data-testid="tool-card"
      className="my-2 rounded-xl border border-slate-300 bg-white/90 p-3 text-slate-900 shadow-sm"
    >
      <div className="flex items-center justify-between gap-2">
        <strong className="font-bold text-slate-900">Tool: {name}</strong>
        <span className="rounded-full border border-slate-400 bg-slate-100 px-2 py-0.5 text-xs font-semibold text-slate-700">
          {state}
        </span>
      </div>

      {input != null && (
        <div className="mt-2">
          <button
            className="rounded-lg border border-slate-300 bg-white px-3 py-1.5 text-sm font-semibold text-slate-800 transition hover:bg-slate-50"
            onClick={() => setShowInput((p) => !p)}
          >
            {showInput ? "Hide input" : "Show input"}
          </button>
          {showInput && (
            <pre className="mt-2 overflow-x-auto rounded-lg border border-slate-200 bg-slate-50 p-2 font-mono text-xs text-slate-700">
              {JSON.stringify(input, null, 2)}
            </pre>
          )}
        </div>
      )}

      {output != null && (
        <div className="mt-2">
          <button
            className="rounded-lg border border-slate-300 bg-white px-3 py-1.5 text-sm font-semibold text-slate-800 transition hover:bg-slate-50"
            onClick={() => setShowOutput((p) => !p)}
          >
            {showOutput ? "Hide output" : "Show output"}
          </button>
          {showOutput && (
            <pre className="mt-2 overflow-x-auto rounded-lg border border-slate-200 bg-slate-50 p-2 font-mono text-xs text-slate-700">
              {typeof output === "string" ? output : JSON.stringify(output, null, 2)}
            </pre>
          )}
        </div>
      )}

      {errorText && (
        <div className="mt-2 text-sm text-red-600">Error: {errorText}</div>
      )}
    </div>
  );
}
