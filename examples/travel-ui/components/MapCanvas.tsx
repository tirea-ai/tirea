"use client";

import { useEffect, useRef, useState } from "react";
import type { Trip } from "@/lib/types";

interface MapCanvasProps {
  trip: Trip | null;
}

export function MapCanvas({ trip }: MapCanvasProps) {
  const mapRef = useRef<HTMLDivElement>(null);
  const [mapInstance, setMapInstance] = useState<any>(null);

  // Dynamically import Leaflet (SSR-safe)
  useEffect(() => {
    if (!mapRef.current || mapInstance) return;

    import("leaflet").then((L) => {
      // Import default Leaflet CSS
      const link = document.createElement("link");
      link.rel = "stylesheet";
      link.href = "https://unpkg.com/leaflet@1.9.4/dist/leaflet.css";
      document.head.appendChild(link);

      const map = L.map(mapRef.current!, { center: [48.8566, 2.3522], zoom: 4 });
      L.tileLayer("https://{s}.tile.openstreetmap.org/{z}/{x}/{y}.png", {
        attribution: "&copy; OpenStreetMap contributors",
      }).addTo(map);
      setMapInstance({ map, L, markers: [] as any[] });
    });

    return () => {
      mapInstance?.map?.remove();
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // Update markers when trip changes
  useEffect(() => {
    if (!mapInstance) return;
    const { map, L, markers } = mapInstance;

    // Clear existing markers
    markers.forEach((m: any) => m.remove());
    mapInstance.markers = [];

    if (!trip || trip.places.length === 0) return;

    const bounds: [number, number][] = [];
    for (const place of trip.places) {
      if (place.lat === 0 && place.lng === 0) continue;
      const marker = L.marker([place.lat, place.lng])
        .addTo(map)
        .bindPopup(`<b>${place.name}</b><br/>${place.description}`);
      mapInstance.markers.push(marker);
      bounds.push([place.lat, place.lng]);
    }

    if (bounds.length > 0) {
      map.fitBounds(bounds, { padding: [50, 50] });
    }
  }, [trip, mapInstance]);

  // Listen for highlight events from frontend tools
  useEffect(() => {
    const handler = (e: Event) => {
      if (!mapInstance) return;
      const { lat, lng } = (e as CustomEvent).detail;
      mapInstance.map.setView([lat, lng], 14);
    };
    window.addEventListener("highlight-place", handler);
    return () => window.removeEventListener("highlight-place", handler);
  }, [mapInstance]);

  return (
    <div
      ref={mapRef}
      style={{ width: "100%", height: "100%", background: "#f0f0f0" }}
    />
  );
}
