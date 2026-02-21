"use client";

import { useChat, type UIMessage } from "@ai-sdk/react";
import { DefaultChatTransport } from "ai";
import { useState, useEffect, useRef, useMemo, useCallback, FormEvent } from "react";

interface InferenceMetrics {
  model: string;
  usage?: {
    prompt_tokens?: number;
    completion_tokens?: number;
    total_tokens?: number;
  };
  duration_ms: number;
}

function getSessionId(): string {
  if (typeof window === "undefined") return "";
  let id = localStorage.getItem("tirea-session-id");
  if (!id) {
    id = `ai-sdk-${crypto.randomUUID()}`;
    localStorage.setItem("tirea-session-id", id);
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
        <h1>Tirea Chat</h1>
        <div style={{ color: "#888" }}>Loading...</div>
      </main>
    );
  }

  // Read agentId from URL query parameter.
  const agentId =
    typeof window !== "undefined"
      ? new URLSearchParams(window.location.search).get("agentId") ?? undefined
      : undefined;

  return <ChatUI sessionId={sessionId} initialMessages={initialMessages} agentId={agentId} />;
}

function ChatUI({
  sessionId,
  initialMessages,
  agentId,
}: {
  sessionId: string;
  initialMessages?: UIMessage[];
  agentId?: string;
}) {
  const transport = useMemo(
    () =>
      new DefaultChatTransport({
        headers: { "x-session-id": sessionId },
        ...(agentId ? { body: { agentId } } : {}),
      }),
    [sessionId, agentId],
  );
  const [metrics, setMetrics] = useState<InferenceMetrics[]>([]);
  const [askAnswers, setAskAnswers] = useState<Record<string, string>>({});
  const autoSubmittedInteractionIds = useRef<Set<string>>(new Set());

  const onData = useCallback((dataPart: { type: string; data: unknown }) => {
    if (dataPart.type === "data-inference-complete") {
      setMetrics((prev) => [...prev, dataPart.data as InferenceMetrics]);
    }
  }, []);

  const sendAutomaticallyWhen = useCallback(
    ({ messages }: { messages: UIMessage[] }) => {
      const last = messages[messages.length - 1];
      if (!last || last.role !== "assistant") {
        return false;
      }

      const interactionIds = last.parts.flatMap((part) => {
        if (
          (part.type === "dynamic-tool" || part.type.startsWith("tool-")) &&
          "state" in part &&
          typeof part.state === "string"
        ) {
          const state = part.state;
          const toolCallId =
            "toolCallId" in part && typeof part.toolCallId === "string"
              ? part.toolCallId
              : undefined;
          const toolName =
            "toolName" in part && typeof part.toolName === "string"
              ? part.toolName
              : part.type.startsWith("tool-")
                ? part.type.slice("tool-".length)
                : undefined;

          if (state === "approval-responded") {
            const approval = (part as { approval?: { id?: unknown } }).approval;
            if (approval && typeof approval.id === "string") {
              return [approval.id];
            }
            if (toolCallId) {
              return [toolCallId];
            }
          }

          if (
            toolName === "askUserQuestion" &&
            (state === "output-available" || state === "output-denied" || state === "output-error")
          ) {
            if (toolCallId) {
              return [toolCallId];
            }
          }
        }
        return [];
      });

      let shouldSend = false;
      for (const interactionId of interactionIds) {
        if (!autoSubmittedInteractionIds.current.has(interactionId)) {
          autoSubmittedInteractionIds.current.add(interactionId);
          shouldSend = true;
        }
      }
      return shouldSend;
    },
    [],
  );

  const { messages, sendMessage, status, error, addToolApprovalResponse, addToolOutput } = useChat({
    messages: initialMessages,
    transport,
    onData: onData as never,
    sendAutomaticallyWhen,
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
      <h1>Tirea Chat</h1>

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
              if (p.type === "reasoning") {
                const reasoning = p as {
                  text?: string;
                  state?: string;
                };
                return (
                  <div
                    key={i}
                    data-testid="reasoning-part"
                    style={{
                      margin: "0.5rem 0",
                      padding: "0.5rem",
                      background: "#fff7e6",
                      borderLeft: "3px solid #f0a500",
                      borderRadius: 4,
                      fontSize: "0.85em",
                      color: "#7a5a00",
                    }}
                  >
                    <strong>Reasoning</strong>
                    {reasoning.state ? ` (${reasoning.state})` : ""}: {reasoning.text ?? ""}
                  </div>
                );
              }
              if (p.type === "source-url") {
                const source = p as {
                  sourceId?: string;
                  url: string;
                  title?: string;
                };
                return (
                  <div key={i} data-testid="source-url-part" style={{ margin: "0.25rem 0" }}>
                    <strong>Source:</strong>{" "}
                    <a href={source.url} target="_blank" rel="noreferrer">
                      {source.title ?? source.url}
                    </a>
                  </div>
                );
              }
              if (p.type === "source-document") {
                const source = p as {
                  title: string;
                  filename?: string;
                  mediaType?: string;
                };
                return (
                  <div key={i} data-testid="source-document-part" style={{ margin: "0.25rem 0" }}>
                    <strong>Document:</strong> {source.title}
                    {source.filename ? ` (${source.filename})` : ""}
                    {source.mediaType ? ` [${source.mediaType}]` : ""}
                  </div>
                );
              }
              if (p.type === "file") {
                const file = p as { url: string; mediaType?: string };
                return (
                  <div key={i} data-testid="file-part" style={{ margin: "0.25rem 0" }}>
                    <strong>File:</strong>{" "}
                    <a href={file.url} target="_blank" rel="noreferrer">
                      {file.url}
                    </a>
                    {file.mediaType ? ` [${file.mediaType}]` : ""}
                  </div>
                );
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
                  approval?: {
                    id: string;
                    approved?: boolean;
                    reason?: string;
                  };
                };
                const name = tool.toolName ?? p.type.replace("tool-", "");
                const requestedToolName =
                  name === "PermissionConfirm" &&
                  tool.input != null &&
                  typeof tool.input === "object" &&
                  "tool_name" in (tool.input as Record<string, unknown>) &&
                  typeof (tool.input as Record<string, unknown>).tool_name === "string"
                    ? ((tool.input as Record<string, unknown>).tool_name as string)
                    : name;
                const askPrompt =
                  name === "askUserQuestion" &&
                  tool.input != null &&
                  typeof tool.input === "object" &&
                  "message" in (tool.input as Record<string, unknown>) &&
                  typeof (tool.input as Record<string, unknown>).message === "string"
                    ? ((tool.input as Record<string, unknown>).message as string)
                    : "";
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
                    {tool.state === "approval-requested" && tool.approval?.id && (
                      <div
                        data-testid="permission-dialog"
                        style={{
                          marginTop: "0.5rem",
                          padding: "0.5rem",
                          border: "1px solid #ddd",
                          borderRadius: 4,
                          background: "#fff",
                        }}
                      >
                        <div style={{ marginBottom: "0.5rem", color: "#333" }}>
                          Approve tool &apos;{requestedToolName}&apos; execution?
                        </div>
                        <div style={{ display: "flex", gap: "0.5rem" }}>
                          <button
                            data-testid="permission-allow"
                            onClick={() =>
                              addToolApprovalResponse({
                                id: tool.approval!.id,
                                approved: true,
                              })
                            }
                            style={{ padding: "0.35rem 0.7rem" }}
                          >
                            Allow
                          </button>
                          <button
                            data-testid="permission-deny"
                            onClick={() =>
                              addToolApprovalResponse({
                                id: tool.approval!.id,
                                approved: false,
                              })
                            }
                            style={{ padding: "0.35rem 0.7rem" }}
                          >
                            Deny
                          </button>
                        </div>
                      </div>
                    )}
                    {tool.state === "input-available" && name === "askUserQuestion" && (
                      <div
                        data-testid="ask-dialog"
                        style={{
                          marginTop: "0.5rem",
                          padding: "0.5rem",
                          border: "1px solid #ddd",
                          borderRadius: 4,
                          background: "#fff",
                        }}
                      >
                        <div
                          data-testid="ask-question-prompt"
                          style={{ marginBottom: "0.5rem", color: "#333" }}
                        >
                          {askPrompt || "Please provide your answer:"}
                        </div>
                        <div style={{ display: "flex", gap: "0.5rem" }}>
                          <input
                            data-testid="ask-question-input"
                            value={askAnswers[tool.toolCallId] ?? ""}
                            onChange={(e) =>
                              setAskAnswers((prev) => ({
                                ...prev,
                                [tool.toolCallId]: e.target.value,
                              }))
                            }
                            placeholder="Type your answer..."
                            style={{
                              flex: 1,
                              padding: "0.35rem 0.5rem",
                              border: "1px solid #ccc",
                              borderRadius: 4,
                            }}
                          />
                          <button
                            data-testid="ask-question-submit"
                            onClick={async () => {
                              const answer = (askAnswers[tool.toolCallId] ?? "").trim();
                              if (!answer) return;
                              await addToolOutput({
                                tool: name as never,
                                toolCallId: tool.toolCallId,
                                state: "output-available",
                                output: { message: answer } as never,
                              });
                              setAskAnswers((prev) => ({
                                ...prev,
                                [tool.toolCallId]: "",
                              }));
                            }}
                            disabled={isLoading || !(askAnswers[tool.toolCallId] ?? "").trim()}
                            style={{ padding: "0.35rem 0.7rem" }}
                          >
                            Submit
                          </button>
                        </div>
                      </div>
                    )}
                    {tool.state === "output-denied" && (
                      <div
                        data-testid="permission-denied"
                        style={{ color: "#b91c1c", marginTop: "0.25rem" }}
                      >
                        Permission denied
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

      {metrics.length > 0 && (
        <div
          data-testid="metrics-panel"
          style={{
            marginBottom: "1rem",
            padding: "0.5rem",
            background: "#f0f4ff",
            borderRadius: 4,
            fontSize: "0.85em",
          }}
        >
          <strong>Token Usage</strong>
          {metrics.map((m, i) => {
            const totalTokens =
              m.usage?.total_tokens ??
              ((m.usage?.prompt_tokens ?? 0) + (m.usage?.completion_tokens ?? 0));
            return (
              <div key={i} data-testid="metrics-entry" style={{ marginTop: "0.25rem" }}>
                {m.model}: {totalTokens > 0 ? `${totalTokens} tokens` : "no usage data"}{" "}
                ({m.duration_ms}ms)
              </div>
            );
          })}
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
