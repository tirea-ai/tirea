export type StarterAction = {
  id: string;
  scenarioId: string;
  title: string;
  capability: string;
  stack: "v1" | "v2";
  prompt: string;
};

export const RECOMMENDED_ACTIONS: StarterAction[] = [
  {
    id: "v1-shared-state",
    scenarioId: "v1.frontend.shared_state.add_todo",
    title: "Shared State Todo",
    capability: "Shared State",
    stack: "v1",
    prompt: "Use addTodo and append: verify AG-UI event ordering.",
  },
  {
    id: "v1-frontend-action",
    scenarioId: "v1.frontend.actions.theme_todo",
    title: "Theme + Todo",
    capability: "Frontend Actions",
    stack: "v1",
    prompt:
      "Use setThemeColor with #16a34a, then use addTodo with text 'ship starter'.",
  },
  {
    id: "v1-generative-ui",
    scenarioId: "v1.frontend.generative.release_checklist",
    title: "Release Checklist",
    capability: "Generative UI",
    stack: "v1",
    prompt:
      "Call showReleaseChecklist with items ['run tests','tag release'] and then summarize the list.",
  },
  {
    id: "v1-hitl",
    scenarioId: "v1.frontend.hitl.confirm_clear_todos",
    title: "Approval Flow",
    capability: "HITL",
    stack: "v1",
    prompt:
      "Use confirmClearTodos and wait for my approval before clearing todos.",
  },
  {
    id: "v1-backend-weather-stock",
    scenarioId: "backend.weather_stock.multi_tool",
    title: "Weather + Stock",
    capability: "Backend Tool",
    stack: "v1",
    prompt:
      "Use get_weather for Tokyo and get_stock_price for AAPL in one answer.",
  },
  {
    id: "v1-backend-note",
    scenarioId: "backend.append_note",
    title: "Append Note",
    capability: "Backend Tool",
    stack: "v1",
    prompt:
      "Use append_note and append exactly this note: copilotkit-demo-note",
  },
  {
    id: "v1-backend-server-info",
    scenarioId: "backend.server_info",
    title: "Server Info",
    capability: "Backend Tool",
    stack: "v1",
    prompt:
      "Use serverInfo and return the server name and timestamp from tool output.",
  },
  {
    id: "v1-backend-progress",
    scenarioId: "backend.progress_demo",
    title: "Progress Demo",
    capability: "Backend Tool",
    stack: "v1",
    prompt:
      "Use progress_demo now so I can observe progress updates.",
  },
  {
    id: "v1-backend-failing",
    scenarioId: "backend.failing_tool",
    title: "Failure Path",
    capability: "Backend Error",
    stack: "v1",
    prompt:
      "Intentionally call failingTool to demonstrate backend error handling.",
  },
  {
    id: "v2-shared-state",
    scenarioId: "v2.frontend.shared_state.add_todos",
    title: "Canvas Shared State",
    capability: "Shared State",
    stack: "v2",
    prompt: "Add two todos about AG-UI contract verification using replaceCanvasTodos.",
  },
  {
    id: "v2-frontend-tools",
    scenarioId: "v2.frontend.toggle_canvas_theme",
    title: "Canvas Theme Tool",
    capability: "Frontend Actions",
    stack: "v2",
    prompt: "Use toggleCanvasTheme to switch between light and dark mode.",
  },
  {
    id: "v2-pie-chart",
    scenarioId: "v2.frontend.generative.pie_chart",
    title: "Pie Chart Card",
    capability: "Generative UI",
    stack: "v2",
    prompt: "Call pieChart with title 'Sprint Mix' and items: dev 5, test 3, docs 2.",
  },
  {
    id: "v2-bar-chart",
    scenarioId: "v2.frontend.generative.bar_chart",
    title: "Bar Chart Card",
    capability: "Generative UI",
    stack: "v2",
    prompt: "Call barChart with title 'Weekly Load' and items: mon 2, tue 4, wed 3.",
  },
  {
    id: "v2-hitl",
    scenarioId: "v2.frontend.hitl.schedule_time",
    title: "Schedule Approval",
    capability: "HITL",
    stack: "v2",
    prompt: "Schedule a meeting with reason 'Review canvas UX' and duration 45.",
  },
  {
    id: "v2-backend-weather-stock",
    scenarioId: "backend.weather_stock.multi_tool",
    title: "Weather + Stock",
    capability: "Backend Tool",
    stack: "v2",
    prompt:
      "Use get_weather for Tokyo and get_stock_price for AAPL in one answer.",
  },
  {
    id: "v2-backend-server-info",
    scenarioId: "backend.server_info",
    title: "Server Info",
    capability: "Backend Tool",
    stack: "v2",
    prompt:
      "Use serverInfo and return the server name and timestamp from tool output.",
  },
];

export const V1_RECOMMENDED_ACTIONS = RECOMMENDED_ACTIONS.filter(
  (action) => action.stack === "v1",
);

export const V2_RECOMMENDED_ACTIONS = RECOMMENDED_ACTIONS.filter(
  (action) => action.stack === "v2",
);

export const ALL_RECOMMENDED_SUGGESTIONS = RECOMMENDED_ACTIONS.map((action) => ({
  title: `[${action.stack.toUpperCase()}] ${action.title}`,
  message: action.prompt,
}));

export const V2_RECOMMENDED_SUGGESTIONS = V2_RECOMMENDED_ACTIONS.map((action) => ({
  title: action.title,
  message: action.prompt,
}));

export const V1_RECOMMENDED_SUGGESTIONS = V1_RECOMMENDED_ACTIONS.map((action) => ({
  title: action.title,
  message: action.prompt,
}));
