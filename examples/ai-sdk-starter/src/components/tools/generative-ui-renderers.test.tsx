import { render, screen, waitFor } from "@testing-library/react";
import "@testing-library/jest-dom/vitest";
import { describe, expect, it, vi } from "vitest";

vi.mock("@openuidev/react-lang", () => ({
  Renderer: ({ response }: { response: string }) => (
    <div data-testid="mock-openui-renderer">{response}</div>
  ),
}));

vi.mock("@openuidev/react-ui", () => ({
  openuiLibrary: {},
}));

describe("A2UIPanel", () => {
  it("shows an error panel instead of crashing on invalid A2UI payloads", async () => {
    const { A2UIPanel } = await import("./generative-ui-renderers");
    const onAction = vi.fn();

    render(
      <A2UIPanel
        onAction={onAction}
        messages={[
          {
            surfaceUpdate: {
              surfaceId: "approval",
              components: [
                {
                  id: "priority",
                  component: {
                    MultipleChoice: {
                      choices: [
                        {
                          label: { literalString: "High" },
                          value: "high",
                        },
                      ],
                      label: { literalString: "Priority" },
                      value: { path: "/request/priority" },
                    },
                  },
                },
              ],
            },
          },
          {
            beginRendering: {
              surfaceId: "approval",
              root: "priority",
            },
          },
        ]}
      />,
    );

    await waitFor(() => {
      expect(screen.getByTestId("a2ui-panel-error")).toBeInTheDocument();
    });
    expect(onAction).not.toHaveBeenCalled();
  });

  it("renders a valid official v0.8 surface", async () => {
    const { A2UIPanel } = await import("./generative-ui-renderers");
    render(
      <A2UIPanel
        messages={[
          {
            surfaceUpdate: {
              surfaceId: "approval",
              components: [
                {
                  id: "root",
                  component: {
                    Card: { child: "title" },
                  },
                },
                {
                  id: "title",
                  component: {
                    Text: {
                      usageHint: "h2",
                      text: { literalString: "Expense Approval Request" },
                    },
                  },
                },
              ],
            },
          },
          {
            beginRendering: {
              surfaceId: "approval",
              root: "root",
            },
          },
        ]}
      />,
    );

    await waitFor(() => {
      expect(document.querySelector('[data-surface-id="approval"]')).toBeTruthy();
    });
    expect(screen.queryByTestId("a2ui-panel-error")).not.toBeInTheDocument();
  });
});

describe("JsonRenderPanel", () => {
  it("renders a compiled spec object", async () => {
    const { JsonRenderPanel } = await import("./generative-ui-renderers");

    render(
      <JsonRenderPanel
        data={{
          root: "root",
          elements: {
            root: {
              type: "Card",
              props: { title: "Ops dashboard" },
              children: [],
            },
          },
        }}
      />,
    );

    expect(screen.getByTestId("json-render-panel")).toBeInTheDocument();
    expect(screen.getByText("Card")).toBeInTheDocument();
    expect(screen.getByText("Ops dashboard")).toBeInTheDocument();
  });

  it("renders a valid JSON document carried as a string", async () => {
    const { JsonRenderPanel } = await import("./generative-ui-renderers");

    render(
      <JsonRenderPanel
        data={JSON.stringify({
          root: "root",
          elements: {
            root: {
              type: "Card",
              props: { title: "Quarterly Review" },
              children: [],
            },
          },
        })}
      />,
    );

    expect(screen.getByTestId("json-render-panel")).toBeInTheDocument();
    expect(screen.getByText("Card")).toBeInTheDocument();
    expect(screen.getByText("Quarterly Review")).toBeInTheDocument();
  });

  it("renders a SpecStream string with the official compiler", async () => {
    const { JsonRenderPanel } = await import("./generative-ui-renderers");

    render(
      <JsonRenderPanel
        data={[
          '{"op":"add","path":"/root","value":"root"}',
          '{"op":"add","path":"/elements/root","value":{"type":"Card","props":{"title":"Quarterly Review"},"children":[]}}',
        ].join("\n")}
        isStreaming
      />,
    );

    expect(screen.getByTestId("json-render-panel")).toBeInTheDocument();
    expect(screen.getByText("Card")).toBeInTheDocument();
    expect(screen.getByText("Quarterly Review")).toBeInTheDocument();
  });

  it("shows a pending state for incomplete streaming JSON", async () => {
    const { JsonRenderPanel } = await import("./generative-ui-renderers");

    render(<JsonRenderPanel data='{"root":"root"' isStreaming />);

    expect(screen.getByTestId("json-render-panel-pending")).toBeInTheDocument();
  });
});
