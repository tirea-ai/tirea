"use client";

interface ResearchCanvasProps {
  question: string;
  report: string;
}

export function ResearchCanvas({ question, report }: ResearchCanvasProps) {
  return (
    <div>
      <h1 style={{ fontSize: 24, marginBottom: 8 }}>Research Canvas</h1>
      {question && (
        <div
          style={{
            padding: 12,
            background: "#f0f4ff",
            borderRadius: 8,
            marginBottom: 16,
          }}
        >
          <div style={{ fontSize: 12, color: "#666", marginBottom: 4 }}>
            Research Question
          </div>
          <div style={{ fontWeight: 600 }}>{question}</div>
        </div>
      )}
      {report ? (
        <div
          style={{
            padding: 16,
            background: "#fff",
            border: "1px solid #e0e0e0",
            borderRadius: 8,
            whiteSpace: "pre-wrap",
            lineHeight: 1.6,
          }}
        >
          {report}
        </div>
      ) : (
        <p style={{ color: "#888" }}>
          No report yet. Ask the assistant to research a topic!
        </p>
      )}
    </div>
  );
}
