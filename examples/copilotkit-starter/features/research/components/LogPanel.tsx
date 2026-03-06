"use client";

import type { LogEntry } from "@/features/research/types";

type LogPanelProps = {
  logs: LogEntry[];
};

export function LogPanel({ logs }: LogPanelProps) {
  if (logs.length === 0) return null;

  return (
    <div
      style={{
        maxHeight: 200,
        overflowY: "auto",
        padding: 12,
        background: "#1e1e1e",
        color: "#d4d4d4",
        fontFamily: "monospace",
        fontSize: 12,
        lineHeight: 1.5,
      }}
    >
      {logs.map((log, index) => (
        <div key={`${log.step}-${index}`}>
          <span
            style={{
              color:
                log.level === "error"
                  ? "#f44336"
                  : log.level === "warn"
                    ? "#ff9800"
                    : "#4caf50",
            }}
          >
            [{log.level.toUpperCase()}]
          </span>{" "}
          <span style={{ color: "#888" }}>[{log.step}]</span> {log.message}
        </div>
      ))}
    </div>
  );
}
