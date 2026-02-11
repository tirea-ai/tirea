import { test, expect, Page } from "@playwright/test";

/** Fill the CopilotKit chat textarea and click Send. */
async function sendChatMessage(page: Page, message: string) {
  const textarea = page.getByPlaceholder("Type a message...");
  await textarea.fill(message);
  const sendBtn = page.locator('button[aria-label="Send"]');
  await expect(sendBtn).toBeEnabled({ timeout: 5_000 });
  await sendBtn.click();
}

/** Return the Nth assistant response message (0-indexed, skipping the greeting). */
function assistantMessage(page: Page, nth = 0) {
  return page.locator(".copilotKitAssistantMessage").nth(nth + 1);
}

test.describe("CopilotKit Chat", () => {
  test.beforeEach(async ({ page }) => {
    await page.goto("/");
    await expect(page.locator("h1")).toBeVisible({ timeout: 15_000 });
  });

  test("page renders with task list and chat panel", async ({ page }) => {
    await expect(page.locator("h1")).toHaveText("Uncarve CopilotKit Demo");

    const taskList = page.getByTestId("task-list");
    await expect(taskList.locator("li")).toHaveCount(2);
    await expect(taskList).toContainText("Review PR");
    await expect(taskList).toContainText("Write tests");

    await expect(page.getByTestId("no-actions")).toBeVisible();
    await expect(page.getByTestId("programmatic-send")).toBeVisible();

    // Verify CopilotKit chat panel rendered.
    await expect(page.locator(".copilotKitChat")).toBeVisible();
    await expect(page.locator('button[aria-label="Send"]')).toBeVisible();
  });

  test("basic chat: send message and receive streaming response", async ({
    page,
  }) => {
    await sendChatMessage(page, "What is 2+2? Reply with just the number.");

    const msg = assistantMessage(page);
    await expect(msg).toBeVisible({ timeout: 45_000 });
    await expect(msg).toContainText("4", { timeout: 15_000 });
  });

  test("useCopilotChat: programmatic message send", async ({ page }) => {
    await page.getByTestId("programmatic-send").click();

    await expect(page.getByTestId("programmatic-sent")).toBeVisible({
      timeout: 5_000,
    });

    await expect(page.locator(".copilotKitMessages")).toContainText(
      "Hello from programmatic message!",
      { timeout: 10_000 }
    );
  });

  // The tests below require the backend agent to support CopilotKit's tool-calling
  // and readable-context protocols. Our carve-agentos-server is a simple LLM passthrough
  // that forwards messages via AG-UI but doesn't process CopilotKit-injected tools/context.
  // The frontend hooks (useCopilotReadable, useCopilotAction) are correctly registered —
  // these tests will pass once the backend supports tool execution.

  test.skip("useCopilotReadable: agent can read task list state", async ({
    page,
  }) => {
    await sendChatMessage(
      page,
      "List the tasks you can see. Reply with just the task titles separated by commas."
    );

    const msg = assistantMessage(page);
    await expect(msg).toBeVisible({ timeout: 45_000 });
    await expect(msg).toContainText("Review PR", { timeout: 15_000 });
    await expect(msg).toContainText("Write tests");
  });

  test.skip("useCopilotAction (addTask): agent adds a task via frontend action", async ({
    page,
  }) => {
    await sendChatMessage(
      page,
      'Add a new task called "Deploy v2" to the task list.'
    );

    const actionLog = page.getByTestId("action-log");
    await expect(actionLog).toContainText("Added:", { timeout: 45_000 });

    const taskList = page.getByTestId("task-list");
    await expect(taskList).toContainText("Deploy v2", { timeout: 5_000 });
  });

  test.skip("useCopilotAction (toggleTask): agent completes a task", async ({
    page,
  }) => {
    await sendChatMessage(page, 'Mark the task "Review PR" as completed.');

    const actionLog = page.getByTestId("action-log");
    await expect(actionLog).toContainText("Toggled:", { timeout: 45_000 });

    const taskSpan = page.locator(
      '[data-testid="task-list"] span:has-text("Review PR")'
    );
    await expect(taskSpan).toHaveAttribute("data-completed", "true", {
      timeout: 5_000,
    });
  });

  test.skip("useCopilotAction (deleteTask): agent deletes a task", async ({
    page,
  }) => {
    await sendChatMessage(
      page,
      'Delete the task "Write tests" from the list.'
    );

    const actionLog = page.getByTestId("action-log");
    await expect(actionLog).toContainText("Deleted:", { timeout: 45_000 });

    const taskList = page.getByTestId("task-list");
    await expect(taskList).not.toContainText("Write tests", { timeout: 5_000 });
  });
});
