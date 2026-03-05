import { expect, test, type APIRequestContext, type Page } from "@playwright/test";

test.describe.configure({ timeout: 180_000 });

async function openCopilotV1(page: Page, agentId = "default") {
  await page.goto(`/v1?agentId=${agentId}`);
  await expect(page.getByTestId("page-title")).toContainText("with-tirea", {
    timeout: 30_000,
  });
  await expect(page.getByPlaceholder("Type a message...")).toBeVisible({
    timeout: 30_000,
  });
}

async function sendPrompt(page: Page, prompt: string) {
  const input = page.getByPlaceholder("Type a message...");
  await input.fill(prompt);
  await page.getByRole("button", { name: "Send" }).click();
  await expect(input).toHaveValue("", { timeout: 15_000 });
}

async function fetchCurrentThreadDump(
  page: Page,
  request: APIRequestContext,
): Promise<string> {
  const threadId = await page.evaluate(() =>
    window.localStorage.getItem("with-tirea.main-thread-id"),
  );
  if (typeof threadId !== "string" || threadId.length === 0) return "";

  const response = await request.get(
    `http://127.0.0.1:38080/v1/ag-ui/threads/${encodeURIComponent(
      threadId,
    )}/messages?order=asc&limit=200&visibility=none`,
  );
  if (!response.ok()) {
    return `status:${response.status()}`;
  }
  return JSON.stringify(await response.json());
}

async function expectThreadContains(
  page: Page,
  request: APIRequestContext,
  needle: string,
) {
  await expect.poll(async () => fetchCurrentThreadDump(page, request), {
    timeout: 120_000,
  }).toContain(needle);
}

test.describe("CopilotKit Backend Capabilities", () => {
  test("v1 page exposes scenario ids for backend demos", async ({ page }) => {
    await openCopilotV1(page);
    await expect(
      page.getByTestId("scenario-id-backend.weather_stock.multi_tool").first(),
    ).toBeVisible();
    await expect(page.getByTestId("scenario-id-backend.server_info").first()).toBeVisible();
    await expect(page.getByTestId("scenario-id-backend.failing_tool").first()).toBeVisible();
  });

  test("weather + stock prompts return backend tool calls", async ({
    page,
    request,
  }) => {
    await openCopilotV1(page);
    const weatherStockPrompt =
      "Use get_weather for Tokyo and get_stock_price for AAPL. RUN_WEATHER_TOOL RUN_STOCK_TOOL";
    await sendPrompt(
      page,
      weatherStockPrompt,
    );
    await expectThreadContains(page, request, "RUN_WEATHER_TOOL");
    await expectThreadContains(page, request, "get_weather");
    await expectThreadContains(page, request, "get_stock_price");
  });

  test("append_note and serverInfo prompts return tool-backed results", async ({
    page,
    request,
  }) => {
    await openCopilotV1(page);
    const appendPrompt =
      "Use append_note with note 'copilotkit-backend-e2e-note'. RUN_APPEND_NOTE. After the tool call, repeat that exact note text.";
    await sendPrompt(
      page,
      appendPrompt,
    );
    await expectThreadContains(page, request, "RUN_APPEND_NOTE");
    await expectThreadContains(page, request, "append_note");

    const serverInfoPrompt =
      "Use serverInfo. RUN_SERVER_INFO. Return server name and timestamp from tool output.";
    await sendPrompt(
      page,
      serverInfoPrompt,
    );
    await expectThreadContains(page, request, "RUN_SERVER_INFO");
    await expectThreadContains(page, request, "serverInfo");
  });

  test("progress_demo, failingTool and finish prompts are handled", async ({
    page,
    request,
  }) => {
    await openCopilotV1(page);
    const progressPrompt =
      "Use progress_demo now. RUN_PROGRESS_DEMO. Confirm completion status.";
    await sendPrompt(
      page,
      progressPrompt,
    );
    await expectThreadContains(page, request, "RUN_PROGRESS_DEMO");
    await expectThreadContains(page, request, "progress_demo");

    const failingPrompt =
      "Intentionally call failingTool. RUN_FAILING_TOOL. Report the failure.";
    await sendPrompt(
      page,
      failingPrompt,
    );
    await expectThreadContains(page, request, "RUN_FAILING_TOOL");
    await expectThreadContains(page, request, "failingTool");
    await expectThreadContains(page, request, "Execution failed");

    await openCopilotV1(page, "stopper");
    const finishPrompt =
      "Use finish with summary 'done'. RUN_FINISH_TOOL. Keep response concise.";
    await sendPrompt(
      page,
      finishPrompt,
    );
    await expectThreadContains(page, request, "RUN_FINISH_TOOL");
    await expectThreadContains(page, request, "finish");
  });
});
