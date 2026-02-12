"use client";

import { useCopilotAction } from "@copilotkit/react-core";

export function useDeleteApproval() {
  useCopilotAction({
    name: "delete_resources",
    description: "Delete resources (requires approval)",
    parameters: [
      {
        name: "resource_ids",
        type: "string[]",
        description: "IDs of resources to delete",
      },
    ],
    renderAndWaitForResponse: ({ args, respond }) => {
      const ids = args.resource_ids ?? [];
      return (
        <div
          style={{
            padding: 12,
            margin: 8,
            border: "1px solid #f44336",
            borderRadius: 8,
            background: "#fff5f5",
          }}
        >
          <p style={{ fontWeight: 600, marginBottom: 8 }}>
            Delete {ids.length} resource(s)?
          </p>
          <div style={{ fontSize: 13, color: "#666", marginBottom: 12 }}>
            IDs: {ids.join(", ")}
          </div>
          <div style={{ display: "flex", gap: 8 }}>
            <button
              onClick={() => respond?.("approved")}
              style={{
                padding: "6px 16px",
                background: "#dc2626",
                color: "#fff",
                border: "none",
                borderRadius: 4,
                cursor: "pointer",
              }}
            >
              Confirm Delete
            </button>
            <button
              onClick={() => respond?.("denied")}
              style={{
                padding: "6px 16px",
                background: "#e0e0e0",
                color: "#333",
                border: "none",
                borderRadius: 4,
                cursor: "pointer",
              }}
            >
              Cancel
            </button>
          </div>
        </div>
      );
    },
  });
}
