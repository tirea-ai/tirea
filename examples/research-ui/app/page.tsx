"use client";

import { CopilotChat } from "@copilotkit/react-ui";
import { ResearchCanvas } from "@/components/ResearchCanvas";
import { ResourcePanel } from "@/components/ResourcePanel";
import { LogPanel } from "@/components/LogPanel";
import { useResearchState } from "@/hooks/useResearchState";
import { useResearchContext } from "@/hooks/useResearchContext";
import { useResearchActions } from "@/hooks/useResearchActions";
import { useDeleteApproval } from "@/hooks/useDeleteApproval";

export default function ResearchPage() {
  const { researchQuestion, report, resources, logs } = useResearchState();
  useResearchContext(researchQuestion, resources);
  useResearchActions();
  useDeleteApproval();

  return (
    <div style={{ display: "flex", height: "100vh" }}>
      {/* Left: Resources */}
      <div
        style={{
          width: 280,
          borderRight: "1px solid #e0e0e0",
          overflowY: "auto",
          padding: 16,
        }}
      >
        <ResourcePanel resources={resources} />
      </div>

      {/* Center: Canvas */}
      <div style={{ flex: 1, display: "flex", flexDirection: "column", padding: 16, overflowY: "auto" }}>
        <ResearchCanvas question={researchQuestion} report={report} />
      </div>

      {/* Right: Chat + Logs */}
      <div
        style={{
          width: 380,
          borderLeft: "1px solid #e0e0e0",
          display: "flex",
          flexDirection: "column",
        }}
      >
        <LogPanel logs={logs} />
        <div style={{ flex: 1, minHeight: 300 }}>
          <CopilotChat
            labels={{
              title: "Research Assistant",
              placeholder: "Ask a research question...",
            }}
          />
        </div>
      </div>
    </div>
  );
}
