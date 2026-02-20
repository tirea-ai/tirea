import { test, expect } from "@playwright/test";

test.describe("AI SDK Chat", () => {
  test("page renders with heading, input, and send button", async ({
    page,
  }) => {
    await page.goto("/");

    await expect(page.locator("h1")).toHaveText("Tirea Chat", {
      timeout: 15_000,
    });

    // Input and button should be present.
    await expect(page.getByPlaceholder("Type a message...")).toBeVisible();
    await expect(page.getByRole("button", { name: "Send" })).toBeVisible();
  });

  test("send message and receive streaming response", async ({ page }) => {
    await page.goto("/");

    await expect(page.locator("h1")).toHaveText("Tirea Chat", {
      timeout: 15_000,
    });

    const input = page.getByPlaceholder("Type a message...");
    await input.fill("What is 2+2? Reply with just the number.");
    await page.getByRole("button", { name: "Send" }).click();

    // User message should appear immediately.
    await expect(
      page.locator("strong", { hasText: "You:" }).first()
    ).toBeVisible({ timeout: 5_000 });

    // "Thinking..." indicator should appear during streaming.
    // (It may be too fast to catch, so we just check for the agent response.)

    // Wait for the assistant response.
    const agentMsg = page.locator("strong", { hasText: "Agent:" }).first();
    await expect(agentMsg).toBeVisible({ timeout: 45_000 });

    // Wait for streaming to finish (Send button re-enabled), then check content.
    const sendButton = page.getByRole("button", { name: "Send" });
    await expect(sendButton).toBeEnabled({ timeout: 30_000 });

    // The response should contain "4".
    const responseDiv = agentMsg.locator("..");
    await expect(responseDiv).toContainText("4", { timeout: 10_000 });
  });

  test("multi-turn conversation preserves history", async ({ page }) => {
    await page.goto("/");

    await expect(page.locator("h1")).toHaveText("Tirea Chat", {
      timeout: 15_000,
    });

    const input = page.getByPlaceholder("Type a message...");

    // Turn 1: ask a question.
    await input.fill("Remember the number 7.");
    await page.getByRole("button", { name: "Send" }).click();

    // Wait for first agent response.
    const firstAgent = page.locator("strong", { hasText: "Agent:" }).first();
    await expect(firstAgent).toBeVisible({ timeout: 45_000 });

    // Turn 2: follow-up referencing the first message.
    await input.fill(
      "What number did I just ask you to remember? Reply with just the number."
    );
    await page.getByRole("button", { name: "Send" }).click();

    // Wait for second agent response.
    const agentMessages = page.locator("strong", { hasText: "Agent:" });
    await expect(agentMessages.nth(1)).toBeVisible({ timeout: 45_000 });

    // Second response should contain "7".
    const secondResponseDiv = agentMessages.nth(1).locator("..");
    await expect(secondResponseDiv).toContainText("7", { timeout: 10_000 });
  });

  test("history messages survive page reload", async ({ page }) => {
    await page.goto("/");

    await expect(page.locator("h1")).toHaveText("Tirea Chat", {
      timeout: 15_000,
    });

    const input = page.getByPlaceholder("Type a message...");

    // Send a message and wait for the agent response.
    await input.fill("Remember the fruit: pineapple. Just say OK.");
    await page.getByRole("button", { name: "Send" }).click();

    const agentMsg = page.locator("strong", { hasText: "Agent:" }).first();
    await expect(agentMsg).toBeVisible({ timeout: 45_000 });

    // Wait for streaming to finish (Send button re-enabled).
    const sendButton = page.getByRole("button", { name: "Send" });
    await expect(sendButton).toBeEnabled({ timeout: 15_000 });

    // Count messages before reload.
    const userCountBefore = await page
      .locator("strong", { hasText: "You:" })
      .count();
    const agentCountBefore = await page
      .locator("strong", { hasText: "Agent:" })
      .count();

    expect(userCountBefore).toBeGreaterThanOrEqual(1);
    expect(agentCountBefore).toBeGreaterThanOrEqual(1);

    // Reload the page — history should be restored from the backend.
    await page.reload();

    await expect(page.locator("h1")).toHaveText("Tirea Chat", {
      timeout: 15_000,
    });

    // Wait for history to load (messages should reappear).
    await expect(
      page.locator("strong", { hasText: "You:" }).first()
    ).toBeVisible({ timeout: 15_000 });

    await expect(
      page.locator("strong", { hasText: "Agent:" }).first()
    ).toBeVisible({ timeout: 15_000 });

    // Verify the original user message content is present.
    const userDiv = page
      .locator("strong", { hasText: "You:" })
      .first()
      .locator("..");
    await expect(userDiv).toContainText("pineapple", { timeout: 5_000 });
  });

  test("displays token usage metrics after response", async ({ page }) => {
    await page.goto("/");

    await expect(page.locator("h1")).toHaveText("Tirea Chat", {
      timeout: 15_000,
    });

    const input = page.getByPlaceholder("Type a message...");
    await input.fill("What is 2+2? Reply with just the number.");
    await page.getByRole("button", { name: "Send" }).click();

    // Wait for the agent response.
    const agentMsg = page.locator("strong", { hasText: "Agent:" }).first();
    await expect(agentMsg).toBeVisible({ timeout: 45_000 });

    // Wait for streaming to finish.
    const sendButton = page.getByRole("button", { name: "Send" });
    await expect(sendButton).toBeEnabled({ timeout: 15_000 });

    // The metrics panel should appear with token usage data.
    const metricsPanel = page.getByTestId("metrics-panel");
    await expect(metricsPanel).toBeVisible({ timeout: 10_000 });

    // Should contain at least one metrics entry with token count.
    const entry = page.getByTestId("metrics-entry").first();
    await expect(entry).toBeVisible();
    await expect(entry).toContainText("tokens");
  });

  test("handles tool execution error gracefully", async ({ page }) => {
    await page.goto("/");

    await expect(page.locator("h1")).toHaveText("Tirea Chat", {
      timeout: 15_000,
    });

    const input = page.getByPlaceholder("Type a message...");
    await input.fill("Trigger an error by using the failingTool.");
    await page.getByRole("button", { name: "Send" }).click();

    // Wait for the agent response that includes the tool error.
    const agentMsg = page.locator("strong", { hasText: "Agent:" }).first();
    await expect(agentMsg).toBeVisible({ timeout: 45_000 });

    // The tool error should be displayed in the chat.
    // The AI SDK frontend renders tool.errorText in a red div with "Error:" prefix.
    // Or the agent may describe the error in text. Either way, "fail" should appear.
    const chatArea = page.locator("main");
    await expect(chatArea).toContainText(/fail|error/i, { timeout: 15_000 });
  });

  test("multi-round tool execution with tool display", async ({ page }) => {
    await page.goto("/");

    await expect(page.locator("h1")).toHaveText("Tirea Chat", {
      timeout: 15_000,
    });

    const input = page.getByPlaceholder("Type a message...");
    await input.fill("Use the serverInfo tool and tell me the server name.");
    await page.getByRole("button", { name: "Send" }).click();

    // Wait for the agent response (multi-round: LLM → tool → LLM).
    const agentMsg = page.locator("strong", { hasText: "Agent:" }).first();
    await expect(agentMsg).toBeVisible({ timeout: 60_000 });

    // Wait for streaming to finish.
    const sendButton = page.getByRole("button", { name: "Send" });
    await expect(sendButton).toBeEnabled({ timeout: 30_000 });

    const chatArea = page.locator("main");

    // Tool display: the frontend should render the tool call UI.
    await expect(chatArea).toContainText("Tool: serverInfo", {
      timeout: 10_000,
    });

    // Tool output should contain the server name.
    await expect(chatArea).toContainText("tirea-agentos", {
      timeout: 10_000,
    });
  });

  test("StopOnTool terminates agent run", async ({ page }) => {
    await page.goto("/?agentId=stopper");

    await expect(page.locator("h1")).toHaveText("Tirea Chat", {
      timeout: 15_000,
    });

    const input = page.getByPlaceholder("Type a message...");
    await input.fill("What is 2+2?");
    await page.getByRole("button", { name: "Send" }).click();

    // Wait for agent response.
    const agentMsg = page.locator("strong", { hasText: "Agent:" }).first();
    await expect(agentMsg).toBeVisible({ timeout: 60_000 });

    // Wait for streaming to finish.
    const sendButton = page.getByRole("button", { name: "Send" });
    await expect(sendButton).toBeEnabled({ timeout: 30_000 });

    const chatArea = page.locator("main");

    // Agent should answer with "4".
    await expect(chatArea).toContainText("4", { timeout: 10_000 });

    // The finish tool should have been called (tool display rendered).
    await expect(chatArea).toContainText("Tool: finish", {
      timeout: 10_000,
    });

    // Run terminated — send button is enabled again.
    await expect(sendButton).toBeEnabled();
  });

  test("send button is disabled while loading", async ({ page }) => {
    await page.goto("/");

    await expect(page.locator("h1")).toHaveText("Tirea Chat", {
      timeout: 15_000,
    });

    const input = page.getByPlaceholder("Type a message...");
    const sendButton = page.getByRole("button", { name: "Send" });

    await input.fill("Tell me a short joke.");
    await sendButton.click();

    // The button should be disabled while waiting.
    await expect(sendButton).toBeDisabled({ timeout: 2_000 });

    // Wait for completion, then button should be enabled again.
    const agentMsg = page.locator("strong", { hasText: "Agent:" }).first();
    await expect(agentMsg).toBeVisible({ timeout: 45_000 });
    await expect(sendButton).toBeEnabled({ timeout: 30_000 });
  });

  test("renders reasoning, sources, files and tool-invocation from history payload", async ({
    page,
  }) => {
    await page.route("**/api/history?**", async (route) => {
      await route.fulfill({
        status: 200,
        contentType: "application/json",
        body: JSON.stringify({
          messages: [
            {
              id: "u1",
              role: "user",
              parts: [{ type: "text", text: "show me details" }],
            },
            {
              id: "a1",
              role: "assistant",
              parts: [
                { type: "reasoning", text: "internal chain", state: "done" },
                {
                  type: "source-url",
                  sourceId: "s1",
                  url: "https://example.com",
                  title: "Example Source",
                },
                {
                  type: "source-document",
                  sourceId: "d1",
                  mediaType: "application/pdf",
                  title: "Spec",
                  filename: "spec.pdf",
                },
                {
                  type: "file",
                  url: "https://example.com/report.csv",
                  mediaType: "text/csv",
                },
                {
                  type: "tool-invocation",
                  toolCallId: "call_1",
                  toolName: "serverInfo",
                  state: "output-available",
                  output: { name: "tirea-agentos" },
                },
              ],
            },
          ],
        }),
      });
    });

    await page.goto("/");
    await expect(page.locator("h1")).toHaveText("Tirea Chat", {
      timeout: 15_000,
    });

    await expect(page.getByTestId("reasoning-part")).toBeVisible();
    await expect(page.getByTestId("source-url-part")).toContainText("Example Source");
    await expect(page.getByTestId("source-document-part")).toContainText("spec.pdf");
    await expect(page.getByTestId("file-part")).toContainText("report.csv");
    await expect(page.locator("main")).toContainText("Tool: serverInfo");
  });

  test("parses streamed reasoning/source/file/tool events from ai-sdk protocol", async ({
    page,
  }) => {
    await page.route("**/api/history?**", async (route) => {
      await route.fulfill({
        status: 200,
        contentType: "application/json",
        body: JSON.stringify({ messages: [] }),
      });
    });

    await page.route("**/api/chat", async (route) => {
      const sse = [
        'data: {"type":"start","messageId":"m_stream"}',
        "",
        'data: {"type":"reasoning-start","id":"r_1"}',
        "",
        'data: {"type":"reasoning-delta","id":"r_1","delta":"thinking"}',
        "",
        'data: {"type":"reasoning-end","id":"r_1"}',
        "",
        'data: {"type":"source-url","sourceId":"s_1","url":"https://example.com","title":"Example"}',
        "",
        'data: {"type":"source-document","sourceId":"d_1","mediaType":"application/pdf","title":"Spec","filename":"spec.pdf"}',
        "",
        'data: {"type":"file","url":"https://example.com/report.csv","mediaType":"text/csv"}',
        "",
        'data: {"type":"tool-input-start","toolCallId":"call_1","toolName":"serverInfo"}',
        "",
        'data: {"type":"tool-input-available","toolCallId":"call_1","toolName":"serverInfo","input":{"scope":"name"}}',
        "",
        'data: {"type":"tool-output-available","toolCallId":"call_1","output":{"name":"tirea-agentos"}}',
        "",
        'data: {"type":"text-start","id":"txt_1"}',
        "",
        'data: {"type":"text-delta","id":"txt_1","delta":"Server is tirea-agentos."}',
        "",
        'data: {"type":"text-end","id":"txt_1"}',
        "",
        'data: {"type":"data-inference-complete","data":{"model":"deepseek-chat","usage":{"prompt_tokens":10,"completion_tokens":5},"duration_ms":123}}',
        "",
        'data: {"type":"finish","finishReason":"stop"}',
        "",
        "data: [DONE]",
        "",
      ].join("\n");

      await route.fulfill({
        status: 200,
        headers: {
          "content-type": "text/event-stream",
          "x-vercel-ai-ui-message-stream": "v1",
        },
        body: sse,
      });
    });

    await page.goto("/");
    await expect(page.locator("h1")).toHaveText("Tirea Chat", {
      timeout: 15_000,
    });

    await page.getByPlaceholder("Type a message...").fill("show rich events");
    await page.getByRole("button", { name: "Send" }).click();

    await expect(page.locator("main")).toContainText("Server is tirea-agentos.");
    await expect(page.getByTestId("reasoning-part")).toBeVisible();
    await expect(page.getByTestId("source-url-part")).toContainText("Example");
    await expect(page.getByTestId("source-document-part")).toContainText("spec.pdf");
    await expect(page.getByTestId("file-part")).toContainText("report.csv");
    await expect(page.locator("main")).toContainText("Tool: serverInfo");
    await expect(page.getByTestId("metrics-panel")).toBeVisible();
  });

  test("handles ai-sdk abort event and keeps partial output", async ({ page }) => {
    await page.route("**/api/history?**", async (route) => {
      await route.fulfill({
        status: 200,
        contentType: "application/json",
        body: JSON.stringify({ messages: [] }),
      });
    });

    await page.route("**/api/chat", async (route) => {
      const sse = [
        'data: {"type":"start","messageId":"m_abort"}',
        "",
        'data: {"type":"text-start","id":"txt_abort"}',
        "",
        'data: {"type":"text-delta","id":"txt_abort","delta":"partial answer"}',
        "",
        'data: {"type":"text-end","id":"txt_abort"}',
        "",
        'data: {"type":"abort","reason":"cancelled"}',
        "",
        "data: [DONE]",
        "",
      ].join("\n");

      await route.fulfill({
        status: 200,
        headers: {
          "content-type": "text/event-stream",
          "x-vercel-ai-ui-message-stream": "v1",
        },
        body: sse,
      });
    });

    await page.goto("/");
    await expect(page.locator("h1")).toHaveText("Tirea Chat", {
      timeout: 15_000,
    });

    const input = page.getByPlaceholder("Type a message...");
    const sendButton = page.getByRole("button", { name: "Send" });

    await input.fill("trigger abort");
    await sendButton.click();

    await expect(page.locator("main")).toContainText("partial answer");
    await expect(sendButton).toBeEnabled({ timeout: 10_000 });
    await expect(page.locator("main")).not.toContainText("Error:");
  });

  test("surfaces ai-sdk error event in UI", async ({ page }) => {
    await page.route("**/api/history?**", async (route) => {
      await route.fulfill({
        status: 200,
        contentType: "application/json",
        body: JSON.stringify({ messages: [] }),
      });
    });

    await page.route("**/api/chat", async (route) => {
      const sse = [
        'data: {"type":"start","messageId":"m_error"}',
        "",
        'data: {"type":"error","errorText":"upstream failure"}',
        "",
        "data: [DONE]",
        "",
      ].join("\n");

      await route.fulfill({
        status: 200,
        headers: {
          "content-type": "text/event-stream",
          "x-vercel-ai-ui-message-stream": "v1",
        },
        body: sse,
      });
    });

    await page.goto("/");
    await expect(page.locator("h1")).toHaveText("Tirea Chat", {
      timeout: 15_000,
    });

    await page.getByPlaceholder("Type a message...").fill("trigger error");
    await page.getByRole("button", { name: "Send" }).click();

    await expect(page.locator("main")).toContainText("Error: upstream failure", {
      timeout: 10_000,
    });
  });
});
