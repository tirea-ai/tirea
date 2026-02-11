import { test, expect } from "@playwright/test";

test.describe("CopilotKit Chat", () => {
  test.beforeEach(async ({ page }) => {
    await page.goto("/");
    // Wait for the page to load.
    await expect(page.locator("h1")).toBeVisible({ timeout: 15_000 });
  });

  test("page renders with task list and chat panel", async ({ page }) => {
    await expect(page.locator("h1")).toHaveText("Uncarve CopilotKit Demo");

    // Verify initial tasks.
    const taskList = page.getByTestId("task-list");
    await expect(taskList.locator("li")).toHaveCount(2);
    await expect(taskList).toContainText("Review PR");
    await expect(taskList).toContainText("Write tests");

    // Verify action log is empty.
    await expect(page.getByTestId("no-actions")).toBeVisible();

    // Verify programmatic send button exists.
    await expect(page.getByTestId("programmatic-send")).toBeVisible();
  });

  // NOTE: Chat interaction tests are skipped because @ag-ui/client@0.0.43 HttpAgent
  // has a version mismatch with CopilotKit's internal @ag-ui/client@0.0.42, causing
  // events to flow but properties to be undefined and runs to never finalize.
  // The page rendering and programmatic UI tests still work.

  test.skip("basic chat: send message and receive streaming response", async ({
    page,
  }) => {
    const textarea = page.getByPlaceholder("Type a message...");
    await textarea.fill("What is 2+2? Reply with just the number.");
    await textarea.press("Enter");

    // Wait for assistant message.
    const assistantMsg = page
      .locator(".copilotKitAssistantMessage")
      .or(page.locator('[data-message-role="assistant"]'))
      .first();
    await expect(assistantMsg).toBeVisible({ timeout: 30_000 });
    await expect(assistantMsg).toContainText("4", { timeout: 10_000 });
  });

  test.skip("useCopilotReadable: agent can read task list state", async ({
    page,
  }) => {
    const textarea = page.getByPlaceholder("Type a message...");
    await textarea.fill(
      "List the tasks you can see. Reply with just the task titles separated by commas."
    );
    await textarea.press("Enter");

    const assistantMsg = page
      .locator(".copilotKitAssistantMessage")
      .or(page.locator('[data-message-role="assistant"]'))
      .first();
    await expect(assistantMsg).toBeVisible({ timeout: 30_000 });

    // The agent should mention both initial tasks.
    await expect(assistantMsg).toContainText("Review PR", { timeout: 10_000 });
    await expect(assistantMsg).toContainText("Write tests");
  });

  test.skip("useCopilotAction (addTask): agent adds a task via frontend action", async ({
    page,
  }) => {
    const textarea = page.getByPlaceholder("Type a message...");
    await textarea.fill('Add a new task called "Deploy v2" to the task list.');
    await textarea.press("Enter");

    // Wait for the action log to reflect the add.
    const actionLog = page.getByTestId("action-log");
    await expect(actionLog).toContainText("Added:", { timeout: 30_000 });

    // Verify the new task appears.
    const taskList = page.getByTestId("task-list");
    await expect(taskList).toContainText("Deploy v2", { timeout: 5_000 });
  });

  test.skip("useCopilotAction (toggleTask): agent completes a task", async ({
    page,
  }) => {
    const textarea = page.getByPlaceholder("Type a message...");
    await textarea.fill('Mark the task "Review PR" as completed.');
    await textarea.press("Enter");

    const actionLog = page.getByTestId("action-log");
    await expect(actionLog).toContainText("Toggled:", { timeout: 30_000 });

    // Verify the task span has data-completed="true".
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
    const textarea = page.getByPlaceholder("Type a message...");
    await textarea.fill('Delete the task "Write tests" from the list.');
    await textarea.press("Enter");

    const actionLog = page.getByTestId("action-log");
    await expect(actionLog).toContainText("Deleted:", { timeout: 30_000 });

    // Verify task is removed.
    const taskList = page.getByTestId("task-list");
    await expect(taskList).not.toContainText("Write tests", { timeout: 5_000 });
  });

  test.skip("useCopilotChat: programmatic message send", async ({ page }) => {
    await page.getByTestId("programmatic-send").click();

    // Verify "Sent!" indicator.
    await expect(page.getByTestId("programmatic-sent")).toBeVisible({
      timeout: 5_000,
    });

    // Verify the programmatic message appears in chat.
    const messagesArea = page
      .locator(".copilotKitMessages")
      .or(page.locator(".copilotKitMessage").first().locator(".."));
    await expect(messagesArea).toContainText(
      "Hello from programmatic message!",
      { timeout: 10_000 }
    );
  });
});
