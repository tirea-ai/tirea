import { describe, expect, it } from "vitest";
import {
  extractSurfaceId,
  formatA2uiActionMessage,
  normalizeA2uiMessages,
} from "./a2ui-protocol";

describe("normalizeA2uiMessages", () => {
  it("accepts the v0.8 messages wrapper and filters invalid components", () => {
    const messages = normalizeA2uiMessages({
      messages: [
        {
          surfaceUpdate: {
            surfaceId: "approval",
            components: {
              id: "root",
              component: {
                Card: { child: "title" },
              },
            },
          },
        },
        {
          dataModelUpdate: {
            surfaceId: "approval",
            path: "/request",
            contents: {
              key: "requester",
              valueString: "Alice",
            },
          },
        },
        {
          beginRendering: {
            surfaceId: "approval",
            root: "root",
          },
        },
      ],
    });

    expect(messages).toHaveLength(3);
    expect(extractSurfaceId(messages)).toBe("approval");
    expect(messages[0]?.surfaceUpdate?.components).toEqual([
      {
        id: "root",
        weight: undefined,
        component: {
          Card: { child: "title" },
        },
      },
    ]);
  });

  it("accepts a single top-level v0.8 message", () => {
    const messages = normalizeA2uiMessages({
      beginRendering: {
        surfaceId: "contact",
        root: "root",
      },
    });

    expect(messages).toEqual([
      {
        beginRendering: {
          surfaceId: "contact",
          root: "root",
          styles: undefined,
        },
      },
    ]);
  });

  it("normalizes datetime-local defaults for DateTimeInput bindings", () => {
    const messages = normalizeA2uiMessages([
      {
        surfaceUpdate: {
          surfaceId: "siteVisit",
          components: [
            {
              id: "visit_datetime",
              component: {
                DateTimeInput: {
                  enableDate: true,
                  enableTime: true,
                  value: {
                    path: "/request/visitDateTime",
                  },
                },
              },
            },
          ],
        },
      },
      {
        dataModelUpdate: {
          surfaceId: "siteVisit",
          path: "/request",
          contents: [
            {
              key: "visitDateTime",
              valueString: "2026-04-10T09:00:00Z",
            },
          ],
        },
      },
    ]);

    expect(messages[1]?.dataModelUpdate?.contents[0]?.valueString).toBe(
      "2026-04-10T09:00",
    );
  });
});

describe("formatA2uiActionMessage", () => {
  it("formats official userAction payloads for the chat loop", () => {
    const message = formatA2uiActionMessage({
      userAction: {
        name: "approval.submit",
        surfaceId: "approval",
        sourceComponentId: "submit_button",
        timestamp: "2026-03-31T12:00:00Z",
        context: { requester: "Alice" },
      },
    });

    expect(message).toBe(
      'A2UI action: {"name":"approval.submit","surfaceId":"approval","sourceComponentId":"submit_button","timestamp":"2026-03-31T12:00:00Z","context":{"requester":"Alice"}}',
    );
  });
});
