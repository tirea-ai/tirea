"use client";

import { CopilotChat } from "@copilotkit/react-ui";

export default function Home() {
  return (
    <main style={{ height: "100vh", display: "flex", flexDirection: "column" }}>
      <h1 style={{ padding: "1rem", margin: 0, fontFamily: "system-ui" }}>
        Uncarve CopilotKit Demo
      </h1>
      <div style={{ flex: 1 }}>
        <CopilotChat
          labels={{ title: "Agent Chat", initial: "Ask me anything!" }}
        />
      </div>
    </main>
  );
}
