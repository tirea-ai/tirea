import { render, screen } from "@testing-library/react";
import "@testing-library/jest-dom/vitest";
import type { UIMessage } from "@ai-sdk/react";
import { describe, expect, it, vi } from "vitest";
import { MessageList } from "./message-list";

vi.mock("@openuidev/react-lang", () => ({
  Renderer: ({ response }: { response: string }) => (
    <div data-testid="mock-openui-renderer">{response}</div>
  ),
}));

vi.mock("@openuidev/react-ui", () => ({
  openuiLibrary: {},
}));

function renderMessageList(messages: UIMessage[]) {
  return render(
    <MessageList
      messages={messages}
      isLoading={false}
      askAnswers={{}}
      onAskAnswerChange={() => {}}
      onApprove={() => {}}
      onDeny={() => {}}
      onAskSubmit={() => {}}
      onFrontendToolSubmit={() => {}}
    />,
  );
}

describe("MessageList generative UI history rendering", () => {
  it("renders JSON Render tool output from history when no live snapshot exists", () => {
    renderMessageList([
      {
        id: "m1",
        role: "assistant",
        parts: [
          {
            type: "tool-render_json_ui",
            toolCallId: "call-1",
            state: "output-available",
            output: {
              content: {
                root: "root",
                elements: {
                  root: {
                    type: "Card",
                    props: { title: "Ops Review" },
                    children: [],
                  },
                },
              },
            },
          },
        ],
      } as UIMessage,
    ]);

    expect(screen.getByTestId("json-render-panel")).toBeInTheDocument();
    expect(screen.getByText("Ops Review")).toBeInTheDocument();
  });

  it("renders OpenUI tool output from history when no live snapshot exists", () => {
    renderMessageList([
      {
        id: "m2",
        role: "assistant",
        parts: [
          {
            type: "tool-render_openui_ui",
            toolCallId: "call-2",
            state: "output-available",
            output: {
              content: "root = Card([])",
            },
          },
        ],
      } as UIMessage,
    ]);

    expect(screen.getByTestId("mock-openui-renderer")).toHaveTextContent(
      "root = Card([])",
    );
  });

  it("renders provider-executed preliminary JSON Render output directly from the tool part", () => {
    renderMessageList([
      {
        id: "m3",
        role: "assistant",
        parts: [
          {
            type: "tool-render_json_ui",
            toolCallId: "call-3",
            state: "output-available",
            providerExecuted: true,
            preliminary: true,
            output: '{"root":"root"',
          },
        ],
      } as UIMessage,
    ]);

    expect(screen.getByTestId("json-render-panel-pending")).toBeInTheDocument();
  });

  it("renders provider-executed preliminary SpecStream output directly from the tool part", () => {
    renderMessageList([
      {
        id: "m4",
        role: "assistant",
        parts: [
          {
            type: "tool-render_json_ui",
            toolCallId: "call-4",
            state: "output-available",
            providerExecuted: true,
            preliminary: true,
            output: [
              '{"op":"add","path":"/root","value":"root"}',
              '{"op":"add","path":"/elements/root","value":{"type":"Card","props":{"title":"Ops Review"},"children":[]}}',
            ].join("\n"),
          },
        ],
      } as UIMessage,
    ]);

    expect(screen.getByTestId("json-render-panel")).toBeInTheDocument();
    expect(screen.getByText("Ops Review")).toBeInTheDocument();
  });
});
