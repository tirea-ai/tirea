import { describe, expect, it } from "vitest";
import { createTransport } from "./transport";

type UiMsg = {
  role: string;
  id: string;
  parts?: Array<{
    type: string;
    text?: string;
    state?: string;
    toolCallId?: string;
    output?: unknown;
    approval?: unknown;
  }>;
};

function makeTransport() {
  return createTransport("thread-1", "default") as unknown as {
    prepareSendMessagesRequest?: (arg: {
      messages: UiMsg[];
      trigger?: string;
      messageId?: string;
    }) => { body: { messages: UiMsg[] } };
  };
}

/**
 * Simulate the exact message sequence AI SDK produces when user clicks
 * Approve or Deny on a PermissionConfirm tool call.
 *
 * Flow:
 * 1. User sends prompt → user message
 * 2. Assistant responds with PermissionConfirm tool call → assistant message
 * 3. User clicks Approve/Deny → addToolOutput updates the assistant part state
 * 4. sendAutomaticallyWhen triggers → prepareSendMessagesRequest called
 *
 * The backend's extract_interaction_responses() scans ASSISTANT messages for
 * tool parts with interaction states. If the transport only sends user
 * messages, the backend never sees the approval/denial.
 */
function permissionScenarioMessages(
  approved: boolean,
): UiMsg[] {
  return [
    {
      role: "user",
      id: "u1",
      parts: [{ type: "text", text: "Use the serverInfo tool." }],
    },
    {
      role: "assistant",
      id: "a1",
      parts: [
        {
          type: "tool-PermissionConfirm",
          toolCallId: "fc_perm_1",
          state: approved ? "output-available" : "output-denied",
          output: { approved },
        },
      ],
    },
  ];
}

describe("createTransport", () => {
  // =========================================================================
  // Existing: basic user-message forwarding
  // =========================================================================

  it("sends only the latest user message after assistant", () => {
    const transport = makeTransport();
    const result = transport.prepareSendMessagesRequest!({
      messages: [
        { role: "user", id: "u1", parts: [{ type: "text", text: "old" }] },
        {
          role: "assistant",
          id: "a1",
          parts: [{ type: "text", text: "reply" }],
        },
        { role: "user", id: "u2", parts: [{ type: "text", text: "new" }] },
      ],
    });

    expect(result.body.messages).toHaveLength(1);
    expect(result.body.messages[0]?.id).toBe("u2");
  });

  it("sends all new user messages after the latest assistant message", () => {
    const transport = makeTransport();
    const result = transport.prepareSendMessagesRequest!({
      messages: [
        {
          role: "user",
          id: "u1",
          parts: [{ type: "text", text: "request" }],
        },
        {
          role: "assistant",
          id: "a1",
          parts: [{ type: "text", text: "need approval" }],
        },
        {
          role: "user",
          id: "u2",
          parts: [{ type: "tool-approval-response" }],
        },
      ],
    });

    expect(result.body.messages).toHaveLength(1);
    expect(result.body.messages[0]?.id).toBe("u2");
  });

  // =========================================================================
  // Bug: permission approve/deny must include assistant interaction parts
  // =========================================================================

  it("includes assistant message with output-available state (approve flow)", () => {
    const transport = makeTransport();
    const messages = permissionScenarioMessages(true);
    const result = transport.prepareSendMessagesRequest!({ messages });

    // The assistant message a1 has output-available state — it MUST be
    // included so the backend can extract the approval decision.
    const ids = result.body.messages.map((m) => m.id);
    expect(ids).toContain("a1");
  });

  it("includes assistant message with output-denied state (deny flow)", () => {
    const transport = makeTransport();
    const messages = permissionScenarioMessages(false);
    const result = transport.prepareSendMessagesRequest!({ messages });

    // The assistant message a1 has output-denied state — it MUST be
    // included so the backend can extract the denial decision.
    const ids = result.body.messages.map((m) => m.id);
    expect(ids).toContain("a1");
  });

  it("approve and deny produce different message payloads", () => {
    const transport = makeTransport();

    const approveResult = transport.prepareSendMessagesRequest!({
      messages: permissionScenarioMessages(true),
    });
    const denyResult = transport.prepareSendMessagesRequest!({
      messages: permissionScenarioMessages(false),
    });

    // Both must include the assistant message
    const approveAssistant = approveResult.body.messages.find(
      (m) => m.id === "a1",
    );
    const denyAssistant = denyResult.body.messages.find(
      (m) => m.id === "a1",
    );
    expect(approveAssistant).toBeDefined();
    expect(denyAssistant).toBeDefined();

    // The tool part states must differ
    const approveState = approveAssistant!.parts?.[0]?.state;
    const denyState = denyAssistant!.parts?.[0]?.state;
    expect(approveState).toBe("output-available");
    expect(denyState).toBe("output-denied");
  });

  it("includes assistant with approval-responded state", () => {
    const transport = makeTransport();
    const result = transport.prepareSendMessagesRequest!({
      messages: [
        {
          role: "user",
          id: "u1",
          parts: [{ type: "text", text: "call serverInfo" }],
        },
        {
          role: "assistant",
          id: "a1",
          parts: [
            {
              type: "tool-PermissionConfirm",
              toolCallId: "fc_1",
              state: "approval-responded",
              approval: { id: "fc_1", approved: false, reason: "not safe" },
            },
          ],
        },
      ],
    });

    const ids = result.body.messages.map((m) => m.id);
    expect(ids).toContain("a1");
  });

  it("includes assistant with output-error state", () => {
    const transport = makeTransport();
    const result = transport.prepareSendMessagesRequest!({
      messages: [
        {
          role: "user",
          id: "u1",
          parts: [{ type: "text", text: "hello" }],
        },
        {
          role: "assistant",
          id: "a1",
          parts: [
            {
              type: "tool-askUserQuestion",
              toolCallId: "ask_1",
              state: "output-error",
            },
          ],
        },
      ],
    });

    const ids = result.body.messages.map((m) => m.id);
    expect(ids).toContain("a1");
  });

  // =========================================================================
  // Non-interaction assistant messages must NOT be included
  // =========================================================================

  it("does not include assistant messages without interaction parts", () => {
    const transport = makeTransport();
    const result = transport.prepareSendMessagesRequest!({
      messages: [
        { role: "user", id: "u1", parts: [{ type: "text", text: "hello" }] },
        {
          role: "assistant",
          id: "a1",
          parts: [{ type: "text", text: "reply" }],
        },
        { role: "user", id: "u2", parts: [{ type: "text", text: "next" }] },
      ],
    });

    const ids = result.body.messages.map((m) => m.id);
    expect(ids).not.toContain("a1");
    expect(ids).toEqual(["u2"]);
  });

  it("does not include assistant with pending tool call (no response yet)", () => {
    const transport = makeTransport();
    const result = transport.prepareSendMessagesRequest!({
      messages: [
        { role: "user", id: "u1", parts: [{ type: "text", text: "go" }] },
        {
          role: "assistant",
          id: "a1",
          parts: [
            {
              type: "tool-PermissionConfirm",
              toolCallId: "fc_1",
              state: "approval-requested",
            },
          ],
        },
      ],
    });

    // approval-requested is NOT an interaction response — it means the
    // dialog is showing but user hasn't clicked yet
    const ids = result.body.messages.map((m) => m.id);
    expect(ids).not.toContain("a1");
  });
});
