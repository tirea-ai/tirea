"use client";

import { useCopilotReadable } from "@copilotkit/react-core";
import type { Resource } from "@/lib/types";

export function useResearchContext(
  researchQuestion: string,
  resources: Resource[]
) {
  useCopilotReadable({
    description: "Current research state",
    value: JSON.stringify({
      researchQuestion,
      resourceCount: resources.length,
      resourceTitles: resources.map((r) => r.title),
    }),
  });
}
