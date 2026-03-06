"use client";

import { useEffect, useRef, useState } from "react";
import type { Trip } from "@/features/travel/types";

type MapCanvasProps = {
  trip: Trip | null;
};

type MapState = {
  map: any;
  L: any;
  markers: any[];
};

export function MapCanvas({ trip }: MapCanvasProps) {
  const mapRef = useRef<HTMLDivElement>(null);
  const [mapState, setMapState] = useState<MapState | null>(null);

  useEffect(() => {
    if (!mapRef.current || mapState) return;

    let disposed = false;
    import("leaflet").then((leaflet) => {
      if (disposed || !mapRef.current) return;

      const existingCss = document.querySelector("link[data-leaflet-css='true']");
      if (!existingCss) {
        const link = document.createElement("link");
        link.rel = "stylesheet";
        link.href = "https://unpkg.com/leaflet@1.9.4/dist/leaflet.css";
        link.dataset.leafletCss = "true";
        document.head.appendChild(link);
      }

      const map = leaflet.map(mapRef.current, { center: [48.8566, 2.3522], zoom: 4 });
      leaflet
        .tileLayer("https://{s}.tile.openstreetmap.org/{z}/{x}/{y}.png", {
          attribution: "&copy; OpenStreetMap contributors",
        })
        .addTo(map);
      setMapState({ map, L: leaflet, markers: [] });
    });

    return () => {
      disposed = true;
    };
  }, [mapState]);

  useEffect(() => {
    return () => {
      mapState?.map?.remove();
    };
  }, [mapState]);

  useEffect(() => {
    if (!mapState) return;
    const { map, L, markers } = mapState;

    markers.forEach((marker) => marker.remove());
    mapState.markers = [];

    if (!trip || trip.places.length === 0) return;

    const bounds: [number, number][] = [];
    for (const place of trip.places) {
      if (place.lat === 0 && place.lng === 0) continue;
      const marker = L.marker([place.lat, place.lng])
        .addTo(map)
        .bindPopup(`<b>${place.name}</b><br/>${place.description}`);
      mapState.markers.push(marker);
      bounds.push([place.lat, place.lng]);
    }

    if (bounds.length > 0) {
      map.fitBounds(bounds, { padding: [50, 50] });
    }
  }, [trip, mapState]);

  useEffect(() => {
    const handler = (event: Event) => {
      if (!mapState) return;
      const detail = (event as CustomEvent).detail as { lat: number; lng: number };
      mapState.map.setView([detail.lat, detail.lng], 14);
    };
    window.addEventListener("highlight-place", handler);
    return () => window.removeEventListener("highlight-place", handler);
  }, [mapState]);

  return <div ref={mapRef} style={{ width: "100%", height: "100%", background: "#f0f0f0" }} />;
}
