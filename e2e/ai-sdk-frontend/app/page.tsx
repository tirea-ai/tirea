"use client";

import { useChat } from "@ai-sdk/react";

export default function Chat() {
  const { messages, input, handleInputChange, handleSubmit, isLoading, error } =
    useChat();

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
            <strong>{m.role === "user" ? "You" : "Agent"}:</strong>{" "}
            {m.parts
              .filter((p) => p.type === "text")
              .map((p) => (p as { type: "text"; text: string }).text)
              .join("")}
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
          onChange={handleInputChange}
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
