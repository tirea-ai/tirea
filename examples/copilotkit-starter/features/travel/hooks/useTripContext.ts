"use client";

import { useCopilotReadable } from "@copilotkit/react-core";
import type { Trip } from "@/features/travel/types";

export function useTripContext(trips: Trip[], selectedTripId: string | null) {
  useCopilotReadable({
    description: "Current travel plan state",
    value: JSON.stringify({
      totalTrips: trips.length,
      selectedTripId,
      tripSummaries: trips.map((trip) => ({
        id: trip.id,
        name: trip.name,
        destination: trip.destination,
        placesCount: trip.places.length,
      })),
    }),
  });
}
