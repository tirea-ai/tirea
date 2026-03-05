import { expect, test, type Page } from "@playwright/test";

async function openChat(page: Page, agentId = "default") {
  await page.goto(`/?agentId=${agentId}`);
  await expect(page.getByTestId("page-title")).toContainText(
    "AI SDK Framework Integration Playground",
    { timeout: 30_000 },
  );
  await expect(page.getByPlaceholder("Type a message...")).toBeVisible({
    timeout: 30_000,
  });
  const createThreadButton = page.getByTestId("thread-create");
  if (await createThreadButton.isVisible()) {
    await createThreadButton.click();
  }
}

async function sendPrompt(page: Page, prompt: string) {
  await page.getByPlaceholder("Type a message...").fill(prompt);
  await page.getByRole("button", { name: "Send" }).click();
}

async function waitForRunComplete(page: Page) {
  await expect(page.getByRole("button", { name: "Send" })).toBeEnabled({
    timeout: 120_000,
  });
}

test.describe("AI SDK Backend Capabilities", () => {
  test("backend weather and stock tools are both callable", async ({ page }) => {
    await openChat(page, "default");
    await sendPrompt(
      page,
      "Use get_weather for Tokyo and get_stock_price for AAPL in one answer. Call both tools before replying.",
    );
    await waitForRunComplete(page);

    const main = page.locator("main");
    await expect(main).toContainText("Tool: get_stock_price", { timeout: 30_000 });
    await expect(main).toContainText("Weather in Tokyo", {
      timeout: 30_000,
    });
  });

  test("append_note backend tool executes", async ({ page }) => {
    await openChat(page, "default");
    await sendPrompt(
      page,
      "Use append_note and append exactly this note: backend-e2e-note",
    );
    await waitForRunComplete(page);
    await expect(page.locator("main")).toContainText("Tool: append_note", {
      timeout: 30_000,
    });
  });

  test("permission flow approval executes backend tool", async ({ page }) => {
    await openChat(page, "permission");
    await sendPrompt(
      page,
      "Use the serverInfo tool to return server name and timestamp.",
    );

    const dialog = page.getByTestId("permission-dialog");
    await expect(dialog).toBeVisible({ timeout: 60_000 });
    await dialog.getByTestId("permission-allow").dispatchEvent("click");
    await waitForRunComplete(page);

    const main = page.locator("main");
    await expect(main).toContainText("Tool: serverInfo", { timeout: 30_000 });
    await expect(main).toContainText("output-available", { timeout: 30_000 });
  });

  test("permission flow deny blocks backend tool", async ({ page }) => {
    await openChat(page, "permission");
    await sendPrompt(
      page,
      "Use the serverInfo tool to return server name and timestamp.",
    );

    const dialog = page.getByTestId("permission-dialog");
    await expect(dialog).toBeVisible({ timeout: 60_000 });
    await dialog.getByTestId("permission-deny").dispatchEvent("click");
    await waitForRunComplete(page);

    await expect(page.getByTestId("permission-denied").first()).toBeVisible({
      timeout: 30_000,
    });
  });

  test("askUserQuestion frontend resume roundtrip works", async ({ page }) => {
    await openChat(page, "default");
    await sendPrompt(
      page,
      "Use askUserQuestion and ask me a short question.",
    );

    const dialog = page.getByTestId("ask-dialog");
    await expect(dialog).toBeVisible({ timeout: 60_000 });
    await page.getByTestId("ask-question-input").fill("blue");
    await page.getByTestId("ask-question-submit").click();
    await waitForRunComplete(page);

    await expect(page.locator("main")).toContainText("Tool: askUserQuestion", {
      timeout: 30_000,
    });
    await expect(page.locator("main")).toContainText("output-available", {
      timeout: 30_000,
    });
  });

  test("set_background_color frontend tool updates chat surface", async ({
    page,
  }) => {
    await openChat(page, "default");
    await sendPrompt(
      page,
      "Use set_background_color (not agent_run) and offer colors #dbeafe and #dcfce7.",
    );

    const dialog = page.getByTestId("set-background-color-dialog");
    await expect(dialog).toBeVisible({ timeout: 60_000 });
    await page.getByTestId("set-background-color-option-0").click();
    await waitForRunComplete(page);

    await expect(page.getByTestId("chat-frontend-surface")).toHaveAttribute(
      "style",
      /rgba\(219,\s*234,\s*254/i,
    );
  });

  test("progress, failing and finish tools are handled end-to-end", async ({
    page,
  }) => {
    await openChat(page, "default");
    await sendPrompt(
      page,
      "Use the progress_demo tool now so I can verify progress events.",
    );
    await waitForRunComplete(page);
    await expect(page.getByTestId("tool-progress-panel")).toBeVisible({
      timeout: 30_000,
    });
    await expect(page.locator("main")).toContainText("Tool: progress_demo", {
      timeout: 30_000,
    });

    await sendPrompt(
      page,
      "Intentionally call failingTool to demonstrate an error path.",
    );
    await waitForRunComplete(page);
    await expect(page.locator("main")).toContainText("failingTool", {
      timeout: 30_000,
    });
    await expect(page.locator("main")).toContainText(/error|failed/i, {
      timeout: 30_000,
    });

    await openChat(page, "stopper");
    await sendPrompt(
      page,
      "Use finish with summary 'done' so the run stops immediately.",
    );
    await waitForRunComplete(page);
    await expect(page.locator("main")).toContainText("Tool: finish", {
      timeout: 30_000,
    });
  });
});
