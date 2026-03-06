"use client";

import type { Resource } from "@/features/research/types";

type ResourcePanelProps = {
  resources: Resource[];
};

export function ResourcePanel({ resources }: ResourcePanelProps) {
  return (
    <div>
      <h3 style={{ margin: "0 0 12px", fontSize: 16 }}>Resources ({resources.length})</h3>
      {resources.length === 0 && (
        <p style={{ color: "#888", fontSize: 13 }}>
          No resources yet. The assistant will find them during research.
        </p>
      )}
      {resources.map((resource) => (
        <div
          key={resource.id}
          style={{
            padding: 10,
            marginBottom: 8,
            borderRadius: 6,
            border: "1px solid #e0e0e0",
            background: "#fff",
          }}
        >
          <a
            href={resource.url}
            target="_blank"
            rel="noopener noreferrer"
            style={{ fontWeight: 600, fontSize: 14, color: "#2563eb" }}
          >
            {resource.title}
          </a>
          <div style={{ fontSize: 12, color: "#666", marginTop: 4 }}>{resource.description}</div>
        </div>
      ))}
    </div>
  );
}
