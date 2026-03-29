import { describe, expect, it } from "vitest";
import { createTransport } from "./transport";

type UiMsg = {
  role: string;
  id: string;
  parts?: Array<Record<string, unknown>>;
};

function makeTransport() {
  return createTransport("thread-1", "default") as unknown as {
    prepareSendMessagesRequest?: (arg: {
      messages: UiMsg[];
      trigger?: string;
      messageId?: string;
    }) => { body: { threadId: string; messages: UiMsg[] } };
  };
}

describe("createTransport", () => {
  it("passes all messages through unchanged", () => {
    const transport = makeTransport();
    const messages: UiMsg[] = [
      { role: "user", id: "u1", parts: [{ type: "text", text: "hello" }] },
      { role: "assistant", id: "a1", parts: [{ type: "text", text: "hi" }] },
      { role: "user", id: "u2", parts: [{ type: "text", text: "next" }] },
    ];
    const result = transport.prepareSendMessagesRequest!({ messages });

    expect(result.body.messages).toEqual(messages);
    expect(result.body.messages).toHaveLength(3);
  });

  it("injects threadId into body", () => {
    const transport = makeTransport();
    const result = transport.prepareSendMessagesRequest!({
      messages: [{ role: "user", id: "u1" }],
    });

    expect(result.body.threadId).toBe("thread-1");
  });

  it("does not generate runId", () => {
    const transport = makeTransport();
    const result = transport.prepareSendMessagesRequest!({
      messages: [{ role: "user", id: "u1" }],
    });

    expect(result.body).not.toHaveProperty("runId");
  });

  it("includes assistant messages with tool interaction states", () => {
    const transport = makeTransport();
    const messages: UiMsg[] = [
      { role: "user", id: "u1", parts: [{ type: "text", text: "go" }] },
      {
        role: "assistant",
        id: "a1",
        parts: [
          {
            type: "tool-PermissionConfirm",
            toolCallId: "fc_1",
            state: "output-available",
            output: { approved: true },
          },
        ],
      },
    ];
    const result = transport.prepareSendMessagesRequest!({ messages });

    // All messages pass through — backend handles dedup + decision extraction
    expect(result.body.messages).toHaveLength(2);
    expect(result.body.messages[1]?.id).toBe("a1");
  });
});
