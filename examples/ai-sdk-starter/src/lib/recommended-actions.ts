export type StarterAction = {
  id: string;
  scenarioId: string;
  title: string;
  capability: string;
  prompt: string;
  agentId?:
    | "default"
    | "permission"
    | "stopper"
    | "a2ui"
    | "json-render"
    | "openui";
};

export const RECOMMENDED_ACTIONS: readonly StarterAction[] = [
  {
    id: "a2ui-expense-approval",
    scenarioId: "a2ui.expense_approval",
    title: "Expense Approval",
    capability: "Generative UI",
    prompt:
      "Create an expense approval form for an operations manager. Include requester name, department, amount, business justification, a priority selector, and a submit button. Use realistic business copy.",
    agentId: "a2ui",
  },
  {
    id: "a2ui-site-visit",
    scenarioId: "a2ui.site_visit",
    title: "Site Visit Request",
    capability: "Generative UI",
    prompt:
      "Create a site visit request form for an operations coordinator. Include requester name, facility location, visit date and time, an on-site escort required checkbox, visit purpose, and a submit button. Use realistic business copy.",
    agentId: "a2ui",
  },
  {
    id: "a2ui-vendor-intake",
    scenarioId: "a2ui.vendor_intake",
    title: "Vendor Intake",
    capability: "Generative UI",
    prompt:
      "Create a vendor onboarding intake form for a procurement team. Include supplier name, category, contract start date, payment terms, risk level, and a submit button. Use realistic business copy.",
    agentId: "a2ui",
  },
  {
    id: "json-render-procurement-workspace",
    scenarioId: "json_render.procurement_workspace",
    title: "Procurement Workspace",
    capability: "Generative UI",
    prompt:
      "Create a procurement request workspace for an internal operations team. Include a summary card, current approval status, requester details, line items table, and primary action buttons. Use realistic product copy.",
    agentId: "json-render",
  },
  {
    id: "json-render-kpi-dashboard",
    scenarioId: "json_render.kpi_dashboard",
    title: "KPI Dashboard",
    capability: "Generative UI",
    prompt:
      "Create a quarterly service operations dashboard with KPI summary cards, a trend chart area, a regional performance table, and status badges. Use realistic enterprise copy.",
    agentId: "json-render",
  },
  {
    id: "openui-escalation-review",
    scenarioId: "openui.escalation_review",
    title: "Escalation Review",
    capability: "Generative UI",
    prompt:
      "Create a customer escalation review panel for a support lead. Include a case summary, severity tag, response owner, remediation notes, and action buttons. Use realistic business copy.",
    agentId: "openui",
  },
  {
    id: "openui-compliance-intake",
    scenarioId: "openui.compliance_intake",
    title: "Compliance Intake",
    capability: "Generative UI",
    prompt:
      "Create a compliance incident intake screen for an internal audit team. Include incident type, affected system, detection date, impact summary, current status, and next-step actions. Use realistic business copy.",
    agentId: "openui",
  },
  {
    id: "demo-weather",
    scenarioId: "backend.weather",
    title: "Weather Check",
    capability: "Backend Tool",
    prompt:
      "Use the get_weather tool with location 'Tokyo' and then summarize the result in one sentence.",
    agentId: "default",
  },
  {
    id: "demo-stock",
    scenarioId: "backend.stock",
    title: "Stock Quote",
    capability: "Backend Tool",
    prompt:
      "Use the get_stock_price tool for symbol 'AAPL' and report the returned price.",
    agentId: "default",
  },
  {
    id: "demo-note",
    scenarioId: "backend.append_note",
    title: "Append Note",
    capability: "Stateful Tool",
    prompt:
      "Use the append_note tool and append exactly this note: starter-demo-note",
    agentId: "default",
  },
  {
    id: "demo-server-info",
    scenarioId: "backend.server_info",
    title: "Server Info",
    capability: "Backend Tool",
    prompt:
      "Use the serverInfo tool and return the server name and timestamp from tool output.",
    agentId: "default",
  },
  {
    id: "demo-progress",
    scenarioId: "backend.progress_demo",
    title: "Progress (Default)",
    capability: "Tool Progress",
    prompt:
      "Use the progress_demo tool with scenario 'default' so I can observe tool progress events.",
    agentId: "default",
  },
  {
    id: "demo-progress-pipeline",
    scenarioId: "backend.progress_demo.pipeline",
    title: "Progress (Data Pipeline)",
    capability: "Tool Progress",
    prompt:
      "Use the progress_demo tool with scenario 'data_pipeline' to simulate an ETL data pipeline.",
    agentId: "default",
  },
  {
    id: "demo-progress-deploy",
    scenarioId: "backend.progress_demo.deploy",
    title: "Progress (Deploy)",
    capability: "Tool Progress",
    prompt:
      "Use the progress_demo tool with scenario 'deploy' to simulate a deployment pipeline.",
    agentId: "default",
  },
  {
    id: "demo-progress-build",
    scenarioId: "backend.progress_demo.build",
    title: "Progress (Slow Build)",
    capability: "Tool Progress",
    prompt:
      "Use the progress_demo tool with scenario 'slow_build' to simulate a long compilation.",
    agentId: "default",
  },
  {
    id: "demo-progress-multi",
    scenarioId: "backend.progress_demo.multi_phase",
    title: "Progress (Multi-Phase)",
    capability: "Tool Progress",
    prompt:
      "Use the progress_demo tool with scenario 'multi_phase' to simulate a multi-phase workflow.",
    agentId: "default",
  },
  {
    id: "demo-failing",
    scenarioId: "backend.failing_tool",
    title: "Failure Path",
    capability: "Backend Error",
    prompt:
      "Intentionally call the failingTool to demonstrate backend error handling.",
    agentId: "default",
  },
  {
    id: "demo-ask-user",
    scenarioId: "frontend.ask_user_question",
    title: "Ask User",
    capability: "Frontend Tool",
    prompt:
      "Call the askUserQuestion frontend tool now. Do not answer in plain text first. Ask exactly: What should I do next?",
    agentId: "default",
  },
  {
    id: "demo-bg-color",
    scenarioId: "frontend.set_background_color",
    title: "Background Color",
    capability: "Client-side Action",
    prompt:
      "Call the set_background_color frontend tool now. Do not answer in plain text first. Pass colors ['#dbeafe', '#dcfce7'] so the frontend can let me pick one.",
    agentId: "default",
  },
  {
    id: "demo-permission-allow",
    scenarioId: "hitl.permission.approve",
    title: "Permission Approve",
    capability: "Approval Flow",
    prompt:
      "Call the append_note tool now. Do not answer in plain text first. Append exactly this note: permission approve demo accepted.",
    agentId: "permission",
  },
  {
    id: "demo-permission-deny",
    scenarioId: "hitl.permission.deny",
    title: "Permission Deny",
    capability: "Approval Flow",
    prompt:
      "Call the append_note tool now. Do not answer in plain text first. Append exactly this note: permission deny demo requested.",
    agentId: "permission",
  },
];
