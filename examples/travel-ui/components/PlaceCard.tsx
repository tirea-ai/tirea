"use client";

import type { Place } from "@/lib/types";

interface PlaceCardProps {
  place: Place;
  onMouseEnter?: () => void;
  onClick?: () => void;
}

export function PlaceCard({ place, onMouseEnter, onClick }: PlaceCardProps) {
  return (
    <div
      onMouseEnter={onMouseEnter}
      onClick={onClick}
      style={{
        padding: 10,
        marginBottom: 8,
        borderRadius: 8,
        border: "1px solid #e0e0e0",
        background: "#fff",
        cursor: onClick ? "pointer" : "default",
        transition: "box-shadow 0.2s",
      }}
      onMouseOver={(e) => {
        (e.currentTarget as HTMLDivElement).style.boxShadow = "0 2px 8px rgba(0,0,0,0.12)";
      }}
      onMouseOut={(e) => {
        (e.currentTarget as HTMLDivElement).style.boxShadow = "none";
      }}
    >
      <div style={{ fontWeight: 600, marginBottom: 4 }}>{place.name}</div>
      <div style={{ fontSize: 13, color: "#666", marginBottom: 4 }}>
        {place.address}
      </div>
      <div style={{ fontSize: 13 }}>{place.description}</div>
      <div style={{ fontSize: 12, color: "#999", marginTop: 4 }}>
        {place.category}
      </div>
    </div>
  );
}
