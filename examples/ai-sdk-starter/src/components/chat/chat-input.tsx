import { useState, type FormEvent } from "react";

type ChatInputProps = {
  onSend: (text: string) => void;
  disabled: boolean;
  themeMode?: "light" | "dark";
};

export function ChatInput({
  onSend,
  disabled,
  themeMode = "light",
}: ChatInputProps) {
  const [input, setInput] = useState("");
  const isDark = themeMode === "dark";

  const handleSubmit = (e: FormEvent) => {
    e.preventDefault();
    const text = input.trim();
    if (!text || disabled) return;
    setInput("");
    onSend(text);
  };

  return (
    <form
      onSubmit={handleSubmit}
      className={
        isDark
          ? "flex gap-2 border-t border-slate-700 bg-slate-900/70 px-4 py-3"
          : "flex gap-2 border-t border-slate-200 bg-slate-50 px-4 py-3"
      }
    >
      <input
        data-testid="chat-input"
        value={input}
        onChange={(e) => setInput(e.target.value)}
        placeholder="Type a message..."
        className={
          isDark
            ? "flex-1 rounded-lg border border-slate-600 bg-slate-950 px-3 py-2 text-sm text-slate-100 outline-none ring-cyan-400 focus:ring-2"
            : "flex-1 rounded-lg border border-slate-300 px-3 py-2 text-sm outline-none ring-cyan-300 focus:ring-2"
        }
      />
      <button
        type="submit"
        disabled={disabled}
        className="rounded-lg bg-cyan-700 px-4 py-2 text-sm font-semibold text-white transition hover:bg-cyan-800 disabled:opacity-50"
      >
        Send
      </button>
    </form>
  );
}
