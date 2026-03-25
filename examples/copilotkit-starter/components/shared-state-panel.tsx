"use client";

import { useState } from "react";

type SharedStatePanelProps = {
  label: string;
  title: string;
  prompt: string;
  todos: string[];
  defaultInput: string;
  onAddTodo: (text: string) => void;
  onClearTodos: () => void;
};

export function SharedStatePanel({
  label,
  title,
  prompt,
  todos,
  defaultInput,
  onAddTodo,
  onClearTodos,
}: SharedStatePanelProps) {
  const [localTodo, setLocalTodo] = useState(defaultInput);

  return (
    <section className="rounded-xl border border-slate-200 bg-white/75 p-3">
      <div className="mb-2 inline-flex items-center rounded-full border border-blue-200 bg-blue-50 px-2 py-0.5 text-xs font-bold text-blue-700">
        {label}
      </div>
      <h2 className="mb-2 text-xl font-semibold text-slate-900">{title}</h2>
      <p data-testid="shared-state-prompt" className="mt-2 text-sm text-slate-600">
        Prompt: "{prompt}"
      </p>
      <p data-testid="todo-count" className="mt-2 text-sm text-slate-700">
        Count: {todos.length}
      </p>
      <ul data-testid="todo-list" className="mt-1 list-disc space-y-1 pl-5 text-sm text-slate-700">
        {todos.map((todo, index) => (
          <li key={`${todo}-${index}`}>{todo}</li>
        ))}
      </ul>
      <pre
        data-testid="shared-state-json"
        className="mt-3 overflow-x-auto rounded-lg border border-slate-200 bg-slate-50 p-3 font-mono text-xs text-slate-700"
      >
        {JSON.stringify({ todos }, null, 2)}
      </pre>
      <div className="mt-3 flex flex-wrap items-center gap-2">
        <input
          data-testid="local-todo-input"
          value={localTodo}
          onChange={(event) => setLocalTodo(event.target.value)}
          placeholder="Type a todo"
          className="min-w-[180px] flex-1 rounded-lg border border-slate-300 bg-white px-3 py-2 text-sm text-slate-900 outline-none ring-cyan-300 transition focus:ring-2"
        />
        <button
          data-testid="add-local-todo"
          className="rounded-lg border border-slate-300 bg-white px-3 py-2 text-sm font-semibold text-slate-800 transition hover:-translate-y-px hover:border-slate-400 hover:bg-slate-50"
          onClick={() => {
            const text = localTodo.trim();
            if (!text) return;
            onAddTodo(text);
            setLocalTodo("");
          }}
        >
          Add Todo
        </button>
        <button
          data-testid="clear-local-todos"
          className="rounded-lg border border-red-300 bg-red-50 px-3 py-2 text-sm font-semibold text-red-800 transition hover:-translate-y-px hover:border-red-400 hover:bg-red-100"
          onClick={() => {
            onClearTodos();
          }}
        >
          Clear Todos
        </button>
      </div>
    </section>
  );
}
