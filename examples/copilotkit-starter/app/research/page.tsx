"use client";

import { CopilotKit } from "@copilotkit/react-core";
import { CopilotChat } from "@copilotkit/react-ui";

import { StarterNavTabs } from "@/components/starter-nav-tabs";
import { LogPanel } from "@/features/research/components/LogPanel";
import { ResearchCanvas } from "@/features/research/components/ResearchCanvas";
import { ResourcePanel } from "@/features/research/components/ResourcePanel";
import { useDeleteApproval } from "@/features/research/hooks/useDeleteApproval";
import { useResearchActions } from "@/features/research/hooks/useResearchActions";
import { useResearchContext } from "@/features/research/hooks/useResearchContext";
import { useResearchState } from "@/features/research/hooks/useResearchState";

const RESEARCH_THREAD_ID = "with-awaken-research";

function ResearchWorkspace() {
  const { researchQuestion, report, resources, logs } = useResearchState();
  useResearchContext(researchQuestion, resources);
  useResearchActions();
  useDeleteApproval();

  return (
    <div style={{ minHeight: "100vh", background: "#e2e8f0" }}>
      <div style={{ padding: "20px 24px 12px" }}>
        <h1 data-testid="research-page-title" style={{ margin: 0, fontSize: 28, color: "#0f172a" }}>
          Research Assistant
        </h1>
        <p style={{ margin: "6px 0 0", color: "#334155", fontSize: 14 }}>
          CopilotKit research canvas using the same awaken backend instance.
        </p>
        <StarterNavTabs mode="research" />
      </div>
      <div style={{ display: "flex", height: "calc(100vh - 112px)", borderTop: "1px solid #cbd5e1" }}>
        <div
          style={{
            width: 280,
            borderRight: "1px solid #e2e8f0",
            overflowY: "auto",
            padding: 16,
            background: "#f8fafc",
          }}
        >
          <ResourcePanel resources={resources} />
        </div>

        <div style={{ flex: 1, display: "flex", flexDirection: "column", padding: 16, overflowY: "auto" }}>
          <ResearchCanvas question={researchQuestion} report={report} />
        </div>

        <div
          style={{
            width: 380,
            borderLeft: "1px solid #e2e8f0",
            display: "flex",
            flexDirection: "column",
            background: "#f8fafc",
          }}
        >
          <LogPanel logs={logs} />
          <div style={{ flex: 1, minHeight: 300 }}>
            <CopilotChat
              labels={{ title: "Research Assistant", placeholder: "Ask a research question..." }}
            />
          </div>
        </div>
      </div>
    </div>
  );
}

export default function ResearchPage() {
  return (
    <CopilotKit runtimeUrl="/api/copilotkit" agent="research" threadId={RESEARCH_THREAD_ID}>
      <ResearchWorkspace />
    </CopilotKit>
  );
}
