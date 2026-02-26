"use client";

import { useCopilotAction } from "@copilotkit/react-core";

export function useResearchActions() {
  useCopilotAction({
    name: "open_resource",
    description: "Open a resource URL in a new browser tab",
    parameters: [
      { name: "url", type: "string", description: "URL to open" },
      { name: "title", type: "string", description: "Resource title" },
    ],
    handler: async ({ url, title }) => {
      window.open(url, "_blank");
      return `Opened "${title}" in a new tab`;
    },
  });
}
