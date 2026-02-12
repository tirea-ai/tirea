"use client";

import { useCoAgent } from "@copilotkit/react-core";
import type { ResearchState } from "@/lib/types";

const defaultState: ResearchState = {
  research_question: "",
  report: "",
  resources: [],
  logs: [],
};

export function useResearchState() {
  const { state } = useCoAgent<ResearchState>({
    name: "research",
    initialState: defaultState,
  });

  return {
    researchQuestion: state?.research_question ?? "",
    report: state?.report ?? "",
    resources: state?.resources ?? [],
    logs: state?.logs ?? [],
  };
}
