"use client";

import type { Trip } from "@/lib/types";
import { PlaceCard } from "./PlaceCard";

interface TripSidebarProps {
  trips: Trip[];
  selectedTripId: string | null;
  onSelectTrip?: (tripId: string) => void;
}

function highlightPlace(lat: number, lng: number) {
  window.dispatchEvent(
    new CustomEvent("highlight-place", { detail: { lat, lng } })
  );
}

export function TripSidebar({ trips, selectedTripId, onSelectTrip }: TripSidebarProps) {
  const selectedTrip = trips.find((t) => t.id === selectedTripId);

  return (
    <div style={{ padding: 16, overflowY: "auto", flex: "0 0 auto", maxHeight: "50%" }}>
      <h2 style={{ margin: "0 0 12px", fontSize: 18 }}>
        {selectedTrip ? selectedTrip.name : "My Trips"}
      </h2>
      {trips.length === 0 && (
        <p style={{ color: "#888" }}>
          No trips yet. Ask the assistant to plan a trip!
        </p>
      )}
      {selectedTrip ? (
        <div>
          <p style={{ color: "#666", marginBottom: 12 }}>
            {selectedTrip.destination} &middot; {selectedTrip.places.length} places
          </p>
          {selectedTrip.places.map((place) => (
            <PlaceCard
              key={place.id}
              place={place}
              onMouseEnter={() => highlightPlace(place.lat, place.lng)}
              onClick={() => highlightPlace(place.lat, place.lng)}
            />
          ))}
        </div>
      ) : (
        <ul style={{ listStyle: "none", padding: 0 }}>
          {trips.map((trip) => (
            <li
              key={trip.id}
              onClick={() => onSelectTrip?.(trip.id)}
              style={{
                padding: "8px 12px",
                marginBottom: 4,
                borderRadius: 6,
                background: "#f5f5f5",
                cursor: "pointer",
              }}
            >
              <strong>{trip.name}</strong>
              <br />
              <span style={{ color: "#666", fontSize: 13 }}>
                {trip.destination} &middot; {trip.places.length} places
              </span>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}
