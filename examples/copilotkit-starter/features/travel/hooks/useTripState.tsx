"use client";

import { useCoAgent, useCoAgentStateRender } from "@copilotkit/react-core";
import { ProgressIndicator } from "@/features/travel/components/ProgressIndicator";
import type { TravelState } from "@/features/travel/types";

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

  useCoAgentStateRender<TravelState>({
    name: "travel",
    render: ({ state: current }) => {
      if (current.search_progress && current.search_progress.length > 0) {
        return <ProgressIndicator progress={current.search_progress} />;
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
