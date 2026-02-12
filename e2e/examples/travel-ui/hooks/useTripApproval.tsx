"use client";

import { useCopilotAction } from "@copilotkit/react-core";

export function useTripApproval() {
  useCopilotAction({
    name: "add_trips",
    description: "Add new trips (requires approval)",
    parameters: [
      {
        name: "trips",
        type: "object[]",
        description: "Trips to add",
        attributes: [
          { name: "id", type: "string" },
          { name: "name", type: "string" },
          { name: "destination", type: "string" },
        ],
      },
    ],
    renderAndWaitForResponse: ({ args, respond, status }) => {
      const trips = args.trips ?? [];
      const isDisabled = status !== "executing";
      return (
        <div style={{ padding: 12, border: "1px solid #e0e0e0", borderRadius: 8, margin: 8, background: "#f9f9f9" }}>
          <p style={{ fontWeight: 600, marginBottom: 8 }}>Add {trips.length} trip(s)?</p>
          <ul style={{ margin: "0 0 8px", paddingLeft: 20 }}>
            {trips.map((t: any, i: number) => (
              <li key={i}>{t.name} — {t.destination}</li>
            ))}
          </ul>
          {status === "complete" ? (
            <div style={{ padding: "6px 0", color: "#16a34a", fontWeight: 500 }}>
              Action completed
            </div>
          ) : (
            <div style={{ display: "flex", gap: 8 }}>
              <button
                disabled={isDisabled}
                onClick={() => respond?.("approved")}
                style={{
                  padding: "6px 16px",
                  background: isDisabled ? "#93c5fd" : "#2563eb",
                  color: "#fff",
                  border: "none",
                  borderRadius: 4,
                  cursor: isDisabled ? "not-allowed" : "pointer",
                  opacity: isDisabled ? 0.6 : 1,
                }}
              >
                {status === "inProgress" ? "Loading..." : "Approve"}
              </button>
              <button
                disabled={isDisabled}
                onClick={() => respond?.("denied")}
                style={{
                  padding: "6px 16px",
                  background: isDisabled ? "#fca5a5" : "#dc2626",
                  color: "#fff",
                  border: "none",
                  borderRadius: 4,
                  cursor: isDisabled ? "not-allowed" : "pointer",
                  opacity: isDisabled ? 0.6 : 1,
                }}
              >
                Deny
              </button>
            </div>
          )}
        </div>
      );
    },
  });
}
