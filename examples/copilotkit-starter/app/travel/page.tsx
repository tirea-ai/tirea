"use client";

import { CopilotKit } from "@copilotkit/react-core";
import { CopilotChat } from "@copilotkit/react-ui";

import { StarterNavTabs } from "@/components/starter-nav-tabs";
import { MapCanvas } from "@/features/travel/components/MapCanvas";
import { TripSidebar } from "@/features/travel/components/TripSidebar";
import { useTripActions } from "@/features/travel/hooks/useTripActions";
import { useTripApproval } from "@/features/travel/hooks/useTripApproval";
import { useTripContext } from "@/features/travel/hooks/useTripContext";
import { useTripState } from "@/features/travel/hooks/useTripState";

const TRAVEL_THREAD_ID = "with-tirea-travel";

function TravelWorkspace() {
  const { trips, selectedTripId } = useTripState();
  useTripContext(trips, selectedTripId);
  useTripActions();
  useTripApproval();

  const selectedTrip = trips.find((trip) => trip.id === selectedTripId) ?? null;

  return (
    <div style={{ minHeight: "100vh", background: "#e2e8f0" }}>
      <div style={{ padding: "20px 24px 12px" }}>
        <h1 data-testid="travel-page-title" style={{ margin: 0, fontSize: 28, color: "#0f172a" }}>
          Travel Planner
        </h1>
        <p style={{ margin: "6px 0 0", color: "#334155", fontSize: 14 }}>
          CopilotKit travel canvas using the same tirea backend instance.
        </p>
        <StarterNavTabs mode="travel" />
      </div>
      <div style={{ display: "flex", height: "calc(100vh - 112px)", borderTop: "1px solid #cbd5e1" }}>
        <div style={{ flex: 1, position: "relative" }}>
          <MapCanvas trip={selectedTrip} />
        </div>
        <div
          style={{
            width: 380,
            borderLeft: "1px solid #e2e8f0",
            display: "flex",
            flexDirection: "column",
            background: "#f8fafc",
          }}
        >
          <TripSidebar trips={trips} selectedTripId={selectedTripId} />
          <div style={{ flex: 1, minHeight: 300 }}>
            <CopilotChat labels={{ title: "Travel Assistant", placeholder: "Plan a trip..." }} />
          </div>
        </div>
      </div>
    </div>
  );
}

export default function TravelPage() {
  return (
    <CopilotKit runtimeUrl="/api/copilotkit" agent="travel" threadId={TRAVEL_THREAD_ID}>
      <TravelWorkspace />
    </CopilotKit>
  );
}
