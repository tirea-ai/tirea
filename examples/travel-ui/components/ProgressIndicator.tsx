"use client";

import type { SearchProgress } from "@/lib/types";

interface ProgressIndicatorProps {
  progress: SearchProgress[];
}

export function ProgressIndicator({ progress }: ProgressIndicatorProps) {
  return (
    <div style={{ display: "flex", flexDirection: "column", gap: 8 }}>
      {progress.map((p, i) => (
        <div
          key={i}
          style={{
            display: "flex",
            alignItems: "center",
            gap: 8,
            padding: "8px 12px",
            borderRadius: 8,
            border: "1px solid #e0e0e0",
            background: "#fff",
          }}
        >
          {p.status === "searching" ? (
            <span
              style={{
                display: "inline-block",
                width: 16,
                height: 16,
                border: "2px solid #ccc",
                borderTopColor: "#2563eb",
                borderRadius: "50%",
                animation: "spin 0.8s linear infinite",
              }}
            />
          ) : (
            <span style={{ color: "#16a34a", fontSize: 16, fontWeight: 700 }}>
              &#x2713;
            </span>
          )}
          <span style={{ fontSize: 13, fontWeight: 500 }}>
            {p.query}
            {p.status !== "searching" && (
              <span style={{ color: "#666", fontWeight: 400 }}>
                {" "}&mdash; Found {p.results_count} places
              </span>
            )}
          </span>
        </div>
      ))}
      <style>{`@keyframes spin { to { transform: rotate(360deg); } }`}</style>
    </div>
  );
}
