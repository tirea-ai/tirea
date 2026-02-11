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
            : t
        )
      );
      setActionLog((prev) => [...prev, `Toggled: ${title}`]);
    },
  });

  // useCopilotAction: let the agent delete tasks
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
    handler: async ({ title }: { title: string }) => {
      setTasks((prev) =>
        prev.filter((t) => t.title.toLowerCase() !== title.toLowerCase())
      );
      setActionLog((prev) => [...prev, `Deleted: ${title}`]);
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
        {tasks.length === 0 && (
          <li data-testid="no-tasks">No tasks</li>
        )}
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
    const { TextMessage, MessageRole } = await import(
      "@copilotkit/runtime-client-gql"
    );
    await appendMessage(
      new TextMessage({
        role: MessageRole.User,
        content: "Hello from programmatic message!",
      })
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
        <span data-testid="programmatic-sent" style={{ marginLeft: "0.5rem", color: "green" }}>
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
      <div style={{ width: "50%", overflow: "auto", borderRight: "1px solid #ccc" }}>
        <h1 style={{ padding: "1rem", margin: 0 }}>
          Uncarve CopilotKit Demo
        </h1>
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
