import { useChat } from "@ai-sdk/react";
import { DefaultChatTransport } from "ai";
import { useRef, useEffect, useState, type FormEvent } from "react";

const BACKEND_URL =
  import.meta.env.VITE_BACKEND_URL ?? "http://localhost:38080";

/**
 * AI Config Assistant — chat interface for AI-driven agent configuration.
 *
 * Uses a dedicated "config-assistant" agent that has tools to:
 * - List/get/create/update agents, models, providers
 * - Read capabilities (available tools, plugins, schemas)
 * - Validate agent configurations
 *
 * The agent can generate complete AgentSpec JSON, suggest plugin configs,
 * and auto-tune parameters based on user intent.
 */
export function AssistantPage() {
  const scrollRef = useRef<HTMLDivElement>(null);
  const [input, setInput] = useState("");

  const { messages, sendMessage, status } = useChat({
    transport: new DefaultChatTransport({
      api: `${BACKEND_URL}/v1/ai-sdk/agents/config-assistant/runs`,
    }),
  });

  // Auto-scroll
  useEffect(() => {
    scrollRef.current?.scrollTo(0, scrollRef.current.scrollHeight);
  }, [messages]);

  const handleSubmit = (e: FormEvent) => {
    e.preventDefault();
    if (!input.trim() || status === "streaming") return;
    sendMessage({ text: input });
    setInput("");
  };

  // Extract text from message parts
  const getMessageText = (msg: (typeof messages)[number]): string => {
    if (!msg.parts) return "";
    return msg.parts
      .filter((p): p is { type: "text"; text: string } => (p as { type: string }).type === "text")
      .map((p) => p.text)
      .join("");
  };

  return (
    <div className="flex flex-col h-full">
      <div className="border-b bg-white px-6 py-3">
        <h2 className="text-lg font-semibold">AI Config Assistant</h2>
        <p className="text-sm text-gray-500">
          Describe the agent you want to create or modify. The assistant will
          generate configurations, suggest plugins, and validate settings.
        </p>
      </div>

      {/* Messages */}
      <div ref={scrollRef} className="flex-1 overflow-auto p-6 space-y-4">
        {messages.length === 0 && (
          <div className="text-center text-gray-400 mt-12 space-y-3">
            <p className="text-lg">What kind of agent would you like to build?</p>
            <div className="flex flex-wrap gap-2 justify-center mt-4">
              {[
                "Create a coding agent with Bash and file tools",
                "Set up a customer support agent with permission controls",
                "Configure a research agent that delegates to sub-agents",
                "Show me all available plugins and their options",
              ].map((suggestion) => (
                <button
                  key={suggestion}
                  onClick={() => setInput(suggestion)}
                  className="px-3 py-1.5 bg-gray-100 hover:bg-gray-200 rounded-full text-sm text-gray-700"
                >
                  {suggestion}
                </button>
              ))}
            </div>
          </div>
        )}

        {messages.map((m) => {
          const text = getMessageText(m);
          if (!text) return null;
          return (
            <div
              key={m.id}
              className={`flex ${
                m.role === "user" ? "justify-end" : "justify-start"
              }`}
            >
              <div
                className={`max-w-[80%] rounded-lg px-4 py-2 text-sm whitespace-pre-wrap ${
                  m.role === "user"
                    ? "bg-blue-600 text-white"
                    : "bg-white shadow text-gray-800"
                }`}
              >
                {text}
              </div>
            </div>
          );
        })}

        {status === "streaming" && (
          <div className="flex justify-start">
            <div className="bg-white shadow rounded-lg px-4 py-2 text-sm text-gray-400 animate-pulse">
              Thinking...
            </div>
          </div>
        )}
      </div>

      {/* Input */}
      <form
        onSubmit={handleSubmit}
        className="border-t bg-white px-6 py-3 flex gap-3"
      >
        <input
          value={input}
          onChange={(e) => setInput(e.target.value)}
          placeholder="Describe your agent or ask about configuration..."
          className="flex-1 px-4 py-2 border rounded-lg text-sm focus:outline-none focus:ring-2 focus:ring-blue-500"
        />
        <button
          type="submit"
          disabled={status === "streaming" || !input.trim()}
          className="px-4 py-2 bg-blue-600 text-white rounded-lg hover:bg-blue-500 disabled:opacity-50 text-sm"
        >
          Send
        </button>
      </form>
    </div>
  );
}
