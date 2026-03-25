type AskUserDialogProps = {
  toolCallId: string;
  prompt: string;
  value: string;
  onChange: (value: string) => void;
  onSubmit: (toolCallId: string, answer: string) => void;
  disabled: boolean;
  themeMode?: "light" | "dark";
};

export function AskUserDialog({
  toolCallId,
  prompt,
  value,
  onChange,
  onSubmit,
  disabled,
  themeMode = "light",
}: AskUserDialogProps) {
  const isDark = themeMode === "dark";
  return (
    <div
      data-testid="ask-dialog"
      className={
        isDark
          ? "mt-2 rounded-lg border border-slate-700 bg-slate-900 p-3"
          : "mt-2 rounded-lg border border-slate-200 bg-white p-3"
      }
    >
      <div
        data-testid="ask-question-prompt"
        className={isDark ? "mb-2 text-sm text-slate-200" : "mb-2 text-sm text-slate-800"}
      >
        {prompt || "Please provide your answer:"}
      </div>
      <div className="flex gap-2">
        <input
          data-testid="ask-question-input"
          value={value}
          onChange={(e) => onChange(e.target.value)}
          placeholder="Type your answer..."
          className={
            isDark
              ? "flex-1 rounded-lg border border-slate-600 bg-slate-950 px-3 py-1.5 text-sm text-slate-100 outline-none ring-cyan-400 focus:ring-2"
              : "flex-1 rounded-lg border border-slate-300 px-3 py-1.5 text-sm outline-none ring-cyan-300 focus:ring-2"
          }
        />
        <button
          data-testid="ask-question-submit"
          onClick={() => {
            const answer = value.trim();
            if (!answer) return;
            onSubmit(toolCallId, answer);
          }}
          disabled={disabled || !value.trim()}
          className="rounded-lg border border-slate-300 bg-white px-3 py-1.5 text-sm font-semibold text-slate-800 transition hover:bg-slate-50 disabled:opacity-50"
        >
          Submit
        </button>
      </div>
    </div>
  );
}
