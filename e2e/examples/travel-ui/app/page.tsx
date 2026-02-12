"use client";

import { CopilotChat } from "@copilotkit/react-ui";
import { MapCanvas } from "@/components/MapCanvas";
import { TripSidebar } from "@/components/TripSidebar";
import { useTripState } from "@/hooks/useTripState";
import { useTripContext } from "@/hooks/useTripContext";
import { useTripActions } from "@/hooks/useTripActions";
import { useTripApproval } from "@/hooks/useTripApproval";

export default function TravelPage() {
  const { trips, selectedTripId } = useTripState();
  useTripContext(trips, selectedTripId);
  useTripActions();
  useTripApproval();

  const selectedTrip = trips.find((t) => t.id === selectedTripId) ?? null;

  return (
    <div style={{ display: "flex", height: "100vh" }}>
      <div style={{ flex: 1, position: "relative" }}>
        <MapCanvas trip={selectedTrip} />
      </div>
      <div
        style={{
          width: 380,
          borderLeft: "1px solid #e0e0e0",
          display: "flex",
          flexDirection: "column",
        }}
      >
        <TripSidebar
          trips={trips}
          selectedTripId={selectedTripId}
        />
        <div style={{ flex: 1, minHeight: 300 }}>
          <CopilotChat
            labels={{ title: "Travel Assistant", placeholder: "Plan a trip..." }}
          />
        </div>
      </div>
    </div>
  );
}
