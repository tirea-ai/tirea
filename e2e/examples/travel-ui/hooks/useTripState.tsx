"use client";

import { useCoAgent, useCoAgentStateRender } from "@copilotkit/react-core";
import { ProgressIndicator } from "@/components/ProgressIndicator";
import type { TravelState } from "@/lib/types";

const defaultState: TravelState = {
  selected_trip_id: null,
  trips: [],
  search_progress: [],
};

export function useTripState() {
  const { state } = useCoAgent<TravelState>({
    name: "travel",
    initialState: defaultState,
  });

  // Render search progress inline in the chat (official CopilotKit pattern)
  useCoAgentStateRender<TravelState>({
    name: "travel",
    render: ({ state }) => {
      if (state.search_progress && state.search_progress.length > 0) {
        return <ProgressIndicator progress={state.search_progress} />;
      }
      return null as any;
    },
  });

  return {
    trips: state?.trips ?? [],
    selectedTripId: state?.selected_trip_id ?? null,
    searchProgress: state?.search_progress ?? [],
  };
}
