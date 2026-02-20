"use client";

import { useState } from "react";
import { CopilotChat } from "@copilotkit/react-ui";
import {
  useCopilotAction,
  useCopilotReadable,
  useCopilotChat,
} from "@copilotkit/react-core";

interface Task {
  id: number;
  title: string;
  completed: boolean;
}

function TaskPanel() {
  const [tasks, setTasks] = useState<Task[]>([
    { id: 1, title: "Review PR", completed: false },
    { id: 2, title: "Write tests", completed: false },
  ]);
  const [actionLog, setActionLog] = useState<string[]>([]);

  // useCopilotReadable: expose task state to the agent
  useCopilotReadable({
    description: "Current task list visible to the user",
    value: tasks,
  });

  // useCopilotAction: let the agent add tasks
  useCopilotAction({
    name: "addTask",
    description: "Add a new task to the task list",
    parameters: [
      {
        name: "title",
        type: "string",
        description: "The task title",
        required: true,
      },
    ],
    handler: async ({ title }: { title: string }) => {
      const newTask = { id: Date.now(), title, completed: false };
      setTasks((prev) => [...prev, newTask]);
      setActionLog((prev) => [...prev, `Added: ${title}`]);
    },
  });

  // useCopilotAction: let the agent toggle task completion
  useCopilotAction({
    name: "toggleTask",
    description: "Mark a task as completed or uncompleted by its title",
    parameters: [
      {
        name: "title",
        type: "string",
        description: "The title of the task to toggle",
        required: true,
      },
    ],
    handler: async ({ title }: { title: string }) => {
      setTasks((prev) =>
        prev.map((t) =>
          t.title.toLowerCase() === title.toLowerCase()
            ? { ...t, completed: !t.completed }
            : t,
        ),
      );
      setActionLog((prev) => [...prev, `Toggled: ${title}`]);
    },
  });

  // PermissionConfirm: handles Permission plugin prompts for backend tool approval
  useCopilotAction({
    name: "PermissionConfirm",
    description: "Confirm permission for a backend tool to execute",
    parameters: [
      {
        name: "tool_name",
        type: "string",
        description: "Name of the tool requesting permission",
        required: true,
      },
      {
        name: "tool_args",
        type: "object",
        description: "Arguments the tool will be called with",
        required: false,
      },
    ],
    renderAndWaitForResponse: ({ args, respond, status }) => {
      const { tool_name, tool_args } = args as { tool_name?: string; tool_args?: Record<string, unknown> };
      const isExecuting = status === "executing";

      if (status === "complete") {
        return (
          <div data-testid="permission-dialog">
            <p style={{ color: "#16a34a", fontWeight: 500 }}>Permission resolved</p>
          </div>
        );
      }

      return (
        <div
          data-testid="permission-dialog"
          style={{
            padding: 12,
            border: "1px solid #e0e0e0",
            borderRadius: 8,
            margin: 8,
            background: "#f9f9f9",
          }}
        >
          <p style={{ fontWeight: 600, marginBottom: 8 }}>
            Allow tool &apos;{tool_name}&apos; to execute?
          </p>
          {tool_args && Object.keys(tool_args).length > 0 && (
            <pre style={{ fontSize: 12, marginBottom: 8, color: "#666" }}>
              {JSON.stringify(tool_args, null, 2)}
            </pre>
          )}
          <div style={{ display: "flex", gap: 8 }}>
            <button
              disabled={!isExecuting}
              data-testid="permission-allow"
              onClick={() => respond?.(JSON.stringify({ approved: true }))}
              style={{
                padding: "6px 16px",
                background: !isExecuting ? "#d1d5db" : "#2563eb",
                color: "#fff",
                border: "none",
                borderRadius: 4,
                cursor: !isExecuting ? "not-allowed" : "pointer",
                opacity: !isExecuting ? 0.6 : 1,
              }}
            >
              Allow
            </button>
            <button
              disabled={!isExecuting}
              data-testid="permission-deny"
              onClick={() => respond?.(JSON.stringify({ approved: false }))}
              style={{
                padding: "6px 16px",
                background: !isExecuting ? "#d1d5db" : "#dc2626",
                color: "#fff",
                border: "none",
                borderRadius: 4,
                cursor: !isExecuting ? "not-allowed" : "pointer",
                opacity: !isExecuting ? 0.6 : 1,
              }}
            >
              Deny
            </button>
          </div>
        </div>
      );
    },
  });

  // useCopilotAction: let the agent delete tasks (HITL — requires approval)
  useCopilotAction({
    name: "deleteTask",
    description: "Delete a task from the task list by its title",
    parameters: [
      {
        name: "title",
        type: "string",
        description: "The title of the task to delete",
        required: true,
      },
    ],
    renderAndWaitForResponse: ({ args, respond, status }) => {
      const title = (args as { title?: string }).title ?? "";
      const isExecuting = status === "executing";

      if (status === "complete") {
        return (
          <div data-testid="delete-approval">
            <p style={{ color: "#16a34a", fontWeight: 500 }}>
              Action completed
            </p>
          </div>
        );
      }

      return (
        <div
          data-testid="delete-approval"
          style={{
            padding: 12,
            border: "1px solid #e0e0e0",
            borderRadius: 8,
            margin: 8,
            background: "#f9f9f9",
          }}
        >
          <p style={{ fontWeight: 600, marginBottom: 8 }}>
            Delete task &quot;{title}&quot;?
          </p>
          <div style={{ display: "flex", gap: 8 }}>
            <button
              disabled={!isExecuting}
              onClick={() => {
                setTasks((prev) =>
                  prev.filter(
                    (t) => t.title.toLowerCase() !== title.toLowerCase(),
                  ),
                );
                setActionLog((prev) => [...prev, `Deleted: ${title}`]);
                respond?.("approved");
              }}
              style={{
                padding: "6px 16px",
                background: !isExecuting ? "#93c5fd" : "#2563eb",
                color: "#fff",
                border: "none",
                borderRadius: 4,
                cursor: !isExecuting ? "not-allowed" : "pointer",
                opacity: !isExecuting ? 0.6 : 1,
              }}
            >
              Approve
            </button>
            <button
              disabled={!isExecuting}
              onClick={() => respond?.("denied")}
              style={{
                padding: "6px 16px",
                background: !isExecuting ? "#fca5a5" : "#dc2626",
                color: "#fff",
                border: "none",
                borderRadius: 4,
                cursor: !isExecuting ? "not-allowed" : "pointer",
                opacity: !isExecuting ? 0.6 : 1,
              }}
            >
              Deny
            </button>
          </div>
        </div>
      );
    },
  });

  return (
    <div style={{ padding: "1rem" }}>
      <h2>Tasks</h2>
      <ul data-testid="task-list">
        {tasks.map((t) => (
          <li key={t.id} data-testid={`task-${t.id}`}>
            <span
              style={{
                textDecoration: t.completed ? "line-through" : "none",
              }}
              data-completed={t.completed}
            >
              {t.title}
            </span>
          </li>
        ))}
        {tasks.length === 0 && <li data-testid="no-tasks">No tasks</li>}
      </ul>

      <h3>Action Log</h3>
      <ul data-testid="action-log">
        {actionLog.map((entry, i) => (
          <li key={i}>{entry}</li>
        ))}
        {actionLog.length === 0 && (
          <li data-testid="no-actions">No actions yet</li>
        )}
      </ul>
    </div>
  );
}

function ProgrammaticControls() {
  const { appendMessage } = useCopilotChat();
  const [sent, setSent] = useState(false);

  const handleSendProgrammatic = async () => {
    const { TextMessage, MessageRole } =
      await import("@copilotkit/runtime-client-gql");
    await appendMessage(
      new TextMessage({
        role: MessageRole.User,
        content: "Hello from programmatic message!",
      }),
    );
    setSent(true);
  };

  return (
    <div style={{ padding: "1rem", borderTop: "1px solid #eee" }}>
      <h3>Programmatic Chat (useCopilotChat)</h3>
      <button
        data-testid="programmatic-send"
        onClick={handleSendProgrammatic}
        style={{ padding: "0.5rem 1rem" }}
      >
        Send Programmatic Message
      </button>
      {sent && (
        <span
          data-testid="programmatic-sent"
          style={{ marginLeft: "0.5rem", color: "green" }}
        >
          Sent!
        </span>
      )}
    </div>
  );
}

export default function Home() {
  return (
    <main
      style={{
        height: "100vh",
        display: "flex",
        fontFamily: "system-ui",
      }}
    >
      {/* Left panel: tasks + controls */}
      <div
        style={{
          width: "50%",
          overflow: "auto",
          borderRight: "1px solid #ccc",
        }}
      >
        <h1 style={{ padding: "1rem", margin: 0 }}>Uncarve CopilotKit Demo</h1>
        <TaskPanel />
        <ProgrammaticControls />
      </div>

      {/* Right panel: chat */}
      <div style={{ width: "50%", display: "flex", flexDirection: "column" }}>
        <CopilotChat
          labels={{ title: "Agent Chat", initial: "Ask me anything!" }}
        />
      </div>
    </main>
  );
}
