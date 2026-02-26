export type StarterAction = {
  id: string;
  title: string;
  capability: string;
  stack: "v1" | "v2";
  prompt: string;
};

export const RECOMMENDED_ACTIONS: StarterAction[] = [
  {
    id: "v1-shared-state",
    title: "Shared State Todo",
    capability: "Shared State",
    stack: "v1",
    prompt: "Add a todo: verify AG-UI event ordering.",
  },
  {
    id: "v1-frontend-action",
    title: "Theme + Todo",
    capability: "Frontend Actions",
    stack: "v1",
    prompt: "Set theme color to #16a34a, then add todo: ship starter.",
  },
  {
    id: "v1-generative-ui",
    title: "Release Checklist",
    capability: "Generative UI",
    stack: "v1",
    prompt: "Call showReleaseChecklist with items: run tests, tag release.",
  },
  {
    id: "v1-hitl",
    title: "Approval Flow",
    capability: "HITL",
    stack: "v1",
    prompt: "Ask me to approve clearing todos before executing.",
  },
  {
    id: "v2-shared-state",
    title: "Canvas Shared State",
    capability: "Shared State",
    stack: "v2",
    prompt: "Add two todos about AG-UI contract verification.",
  },
  {
    id: "v2-frontend-tools",
    title: "Canvas Theme Tool",
    capability: "Frontend Actions",
    stack: "v2",
    prompt: "Toggle the canvas theme.",
  },
  {
    id: "v2-pie-chart",
    title: "Pie Chart Card",
    capability: "Generative UI",
    stack: "v2",
    prompt: "Call pieChart with title 'Sprint Mix' and items: dev 5, test 3, docs 2.",
  },
  {
    id: "v2-bar-chart",
    title: "Bar Chart Card",
    capability: "Generative UI",
    stack: "v2",
    prompt: "Call barChart with title 'Weekly Load' and items: mon 2, tue 4, wed 3.",
  },
  {
    id: "v2-hitl",
    title: "Schedule Approval",
    capability: "HITL",
    stack: "v2",
    prompt: "Schedule a meeting with reason 'Review canvas UX' and duration 45.",
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
