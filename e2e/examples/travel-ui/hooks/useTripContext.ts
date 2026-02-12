"use client";

import { useCopilotReadable } from "@copilotkit/react-core";
import type { Trip } from "@/lib/types";

export function useTripContext(trips: Trip[], selectedTripId: string | null) {
  useCopilotReadable({
    description: "Current travel plan state",
    value: JSON.stringify({
      totalTrips: trips.length,
      selectedTripId,
      tripSummaries: trips.map((t) => ({
        id: t.id,
        name: t.name,
        destination: t.destination,
        placesCount: t.places.length,
      })),
    }),
  });
}
