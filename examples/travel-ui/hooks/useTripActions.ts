"use client";

import { useCopilotAction } from "@copilotkit/react-core";

export function useTripActions() {
  useCopilotAction({
    name: "highlight_place",
    description: "Highlight a place on the map by zooming to it",
    parameters: [
      { name: "place_id", type: "string", description: "The place ID to highlight" },
      { name: "lat", type: "number", description: "Latitude" },
      { name: "lng", type: "number", description: "Longitude" },
    ],
    handler: async ({ place_id, lat, lng }) => {
      // Dispatch a custom event that MapCanvas listens to
      window.dispatchEvent(
        new CustomEvent("highlight-place", {
          detail: { place_id, lat, lng },
        })
      );
      return `Highlighted place ${place_id} at (${lat}, ${lng})`;
    },
  });
}
