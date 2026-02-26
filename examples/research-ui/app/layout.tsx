"use client";

import { CopilotKit } from "@copilotkit/react-core";
import "@copilotkit/react-ui/styles.css";

export default function RootLayout({
  children,
}: {
  children: React.ReactNode;
}) {
  return (
    <html lang="en">
      <body style={{ margin: 0, fontFamily: "system-ui, sans-serif" }}>
        <CopilotKit runtimeUrl="/api/copilotkit" agent="research">
          {children}
        </CopilotKit>
      </body>
    </html>
  );
}
