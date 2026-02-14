"use client";

import { useChat, type UIMessage } from "@ai-sdk/react";
import { DefaultChatTransport } from "ai";
import { useState, useEffect, useRef, useMemo, FormEvent } from "react";

function getSessionId(): string {
  if (typeof window === "undefined") return "";
  let id = localStorage.getItem("uncarve-session-id");
  if (!id) {
    id = `ai-sdk-${crypto.randomUUID()}`;
    localStorage.setItem("uncarve-session-id", id);
  }
  return id;
}

export default function Chat() {
  const [sessionId, setSessionId] = useState("");
  const [initialMessages, setInitialMessages] = useState<UIMessage[] | undefined>(undefined);
  const [historyLoaded, setHistoryLoaded] = useState(false);
  const historyFetched = useRef(false);

  // Initialize sessionId from localStorage on mount.
  useEffect(() => {
    setSessionId(getSessionId());
  }, []);

  // Load history from backend once sessionId is available.
  useEffect(() => {
    if (!sessionId || historyFetched.current) return;
    historyFetched.current = true;

    fetch(`/api/history?sessionId=${encodeURIComponent(sessionId)}`)
      .then((r) => r.json())
      .then((data) => {
        if (data.messages && data.messages.length > 0) {
          setInitialMessages(data.messages);
        } else {
          setInitialMessages(undefined);
        }
        setHistoryLoaded(true);
      })
      .catch(() => {
        setHistoryLoaded(true);
      });
  }, [sessionId]);

  if (!historyLoaded) {
    return (
      <main style={{ maxWidth: 640, margin: "2rem auto", fontFamily: "system-ui" }}>
        <h1>Uncarve Chat</h1>
        <div style={{ color: "#888" }}>Loading...</div>
      </main>
    );
  }

  return <ChatUI sessionId={sessionId} initialMessages={initialMessages} />;
}

function ChatUI({
  sessionId,
  initialMessages,
}: {
  sessionId: string;
  initialMessages?: UIMessage[];
}) {
  const transport = useMemo(
    () => new DefaultChatTransport({ headers: { "x-session-id": sessionId } }),
    [sessionId],
  );
  const { messages, sendMessage, status, error } = useChat({
    messages: initialMessages,
    transport,
  });
  const [input, setInput] = useState("");

  const isLoading = status === "streaming" || status === "submitted";

  const handleSubmit = async (e: FormEvent) => {
    e.preventDefault();
    if (!input.trim() || isLoading) return;
    const text = input;
    setInput("");
    await sendMessage({ text });
  };

  return (
    <main style={{ maxWidth: 640, margin: "2rem auto", fontFamily: "system-ui" }}>
      <h1>Uncarve Chat</h1>

      <div style={{ marginBottom: "1rem" }}>
        {messages.map((m) => (
          <div
            key={m.id}
            style={{
              padding: "0.5rem 0",
              borderBottom: "1px solid #eee",
            }}
          >
            <strong>{m.role === "user" ? "You" : "Agent"}:</strong>
            {m.parts.map((p, i) => {
              if (p.type === "text") {
                return <span key={i}> {p.text}</span>;
              }
              if (p.type === "dynamic-tool" || p.type.startsWith("tool-")) {
                const tool = p as {
                  type: string;
                  toolName?: string;
                  toolCallId: string;
                  state: string;
                  input?: unknown;
                  output?: unknown;
                  errorText?: string;
                };
                const name = tool.toolName ?? p.type.replace("tool-", "");
                return (
                  <div
                    key={i}
                    style={{
                      margin: "0.5rem 0",
                      padding: "0.5rem",
                      background: "#f5f5f5",
                      borderRadius: 4,
                      fontSize: "0.85em",
                      fontFamily: "monospace",
                    }}
                  >
                    <div>
                      <strong>Tool: {name}</strong>{" "}
                      <span style={{ color: "#888" }}>({tool.state})</span>
                    </div>
                    {tool.input != null && (
                      <div style={{ color: "#555", marginTop: "0.25rem" }}>
                        Input: {JSON.stringify(tool.input)}
                      </div>
                    )}
                    {tool.output != null && (
                      <div style={{ color: "#2a7", marginTop: "0.25rem" }}>
                        Output: {JSON.stringify(tool.output)}
                      </div>
                    )}
                    {tool.errorText && (
                      <div style={{ color: "red", marginTop: "0.25rem" }}>
                        Error: {tool.errorText}
                      </div>
                    )}
                  </div>
                );
              }
              return null;
            })}
          </div>
        ))}
        {isLoading && (
          <div style={{ color: "#888", padding: "0.5rem 0" }}>Thinking...</div>
        )}
      </div>

      {error && (
        <div style={{ color: "red", marginBottom: "0.5rem" }}>
          Error: {error.message}
        </div>
      )}

      <form onSubmit={handleSubmit} style={{ display: "flex", gap: "0.5rem" }}>
        <input
          value={input}
          onChange={(e) => setInput(e.target.value)}
          placeholder="Type a message..."
          style={{
            flex: 1,
            padding: "0.5rem",
            border: "1px solid #ccc",
            borderRadius: 4,
          }}
        />
        <button
          type="submit"
          disabled={isLoading}
          style={{ padding: "0.5rem 1rem" }}
        >
          Send
        </button>
      </form>
    </main>
  );
}
