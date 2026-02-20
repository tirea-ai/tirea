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
    // Wait for CopilotKit chat panel to finish async initialization.
    await expect(page.locator('button[aria-label="Send"]')).toBeVisible({
      timeout: 15_000,
    });
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
      { timeout: 10_000 },
    );
  });

  test("useCopilotReadable: agent can read task list state", async ({
    page,
  }) => {
    await sendChatMessage(
      page,
      "List the tasks you can see. Reply with just the task titles separated by commas.",
    );

    const msg = assistantMessage(page);
    await expect(msg).toBeVisible({ timeout: 45_000 });
    await expect(msg).toContainText("Review PR", { timeout: 15_000 });
    await expect(msg).toContainText("Write tests");
  });

  test("useCopilotAction (addTask): agent adds a task via frontend action", async ({
    page,
  }) => {
    await sendChatMessage(
      page,
      'Add a new task called "Deploy v2" to the task list.',
    );

    // Wait for the action log to reflect the add.
    const actionLog = page.getByTestId("action-log");
    await expect(actionLog).toContainText("Added:", { timeout: 45_000 });

    // Verify the new task appears.
    const taskList = page.getByTestId("task-list");
    await expect(taskList).toContainText("Deploy v2", { timeout: 5_000 });
  });

  test("useCopilotAction (toggleTask): agent completes a task", async ({
    page,
  }) => {
    await sendChatMessage(page, 'Mark the task "Review PR" as completed.');

    const actionLog = page.getByTestId("action-log");
    await expect(actionLog).toContainText("Toggled:", { timeout: 45_000 });

    // Verify the task span has data-completed="true".
    const taskSpan = page.locator(
      '[data-testid="task-list"] span:has-text("Review PR")',
    );
    await expect(taskSpan).toHaveAttribute("data-completed", "true", {
      timeout: 5_000,
    });
  });

  test("useCopilotAction (deleteTask): agent deletes a task", async ({
    page,
  }) => {
    await sendChatMessage(page, 'Delete the task "Write tests" from the list.');

    // HITL: wait for the approval dialog, then approve.
    const approval = page.getByTestId("delete-approval");
    const approveBtn = approval.getByRole("button", { name: "Approve" });
    await expect(approveBtn).toBeVisible({ timeout: 45_000 });
    await approveBtn.click();

    const actionLog = page.getByTestId("action-log");
    await expect(actionLog).toContainText("Deleted:", { timeout: 15_000 });

    // Verify task is removed.
    const taskList = page.getByTestId("task-list");
    await expect(taskList).not.toContainText("Write tests", { timeout: 5_000 });
  });

  // --- Multi-round tool chain tests ---

  test("multi-round tool chain: add task updates state", async ({ page }) => {
    await sendChatMessage(
      page,
      'Add a task called "Deploy staging" to the task list.',
    );

    // Wait for the action log to reflect the add.
    const actionLog = page.getByTestId("action-log");
    await expect(actionLog).toContainText("Added:", { timeout: 45_000 });

    // Verify the new task appears in the list (state update chain complete).
    const taskList = page.getByTestId("task-list");
    await expect(taskList).toContainText("Deploy staging", { timeout: 5_000 });

    // Total tasks should now be 3 (original 2 + new 1).
    await expect(taskList.locator("li")).toHaveCount(3, { timeout: 5_000 });
  });

  // --- HITL (Human-in-the-Loop) tests for deleteTask ---

  test("HITL: approval dialog appears for deleteTask", async ({ page }) => {
    await sendChatMessage(page, 'Delete the task "Review PR" from the list.');

    const approval = page.getByTestId("delete-approval");
    await expect(approval).toBeVisible({ timeout: 45_000 });

    // Both buttons present and enabled.
    const approveBtn = approval.getByRole("button", { name: "Approve" });
    const denyBtn = approval.getByRole("button", { name: "Deny" });
    await expect(approveBtn).toBeVisible();
    await expect(denyBtn).toBeVisible();
    await expect(approveBtn).toBeEnabled();
    await expect(denyBtn).toBeEnabled();

    // Dialog mentions the task being deleted.
    await expect(approval).toContainText("Review PR");

    // Clean up: approve so the agent can finish.
    await approveBtn.click();
  });

  test("HITL: denial path — agent continues after deny", async ({ page }) => {
    await sendChatMessage(page, 'Delete the task "Review PR" from the list.');

    const approval = page.getByTestId("delete-approval");
    const denyBtn = approval.getByRole("button", { name: "Deny" });
    await expect(denyBtn).toBeVisible({ timeout: 45_000 });

    await denyBtn.click();

    // After denial the task must still exist.
    const taskList = page.getByTestId("task-list");
    await expect(taskList).toContainText("Review PR", { timeout: 10_000 });

    // Action log should NOT contain "Deleted:".
    const actionLog = page.getByTestId("action-log");
    // Wait for agent follow-up message to confirm it processed the denial.
    const msgs = page.locator(".copilotKitAssistantMessage");
    await expect(msgs.last()).toBeVisible({ timeout: 30_000 });
    await expect(actionLog).not.toContainText("Deleted:");
  });

  // --- Permission plugin tests for backend tool (serverInfo) ---
  // SKIPPED: CopilotKit's AG-UI runtime only renders renderAndWaitForResponse
  // for LLM-initiated tool calls. Backend-initiated frontend tool invocations
  // (from the Permission plugin via invoke_frontend_tool) are not rendered.
  // The backend infrastructure works correctly (verified via direct AG-UI curl).

  test.skip("permission approval allows backend tool execution", async ({
    page,
  }) => {
    await sendChatMessage(
      page,
      "Use the serverInfo tool to get server information.",
    );

    // Wait for the permission dialog to appear.
    const dialog = page.getByTestId("permission-dialog");
    await expect(dialog).toBeVisible({ timeout: 45_000 });

    // Should mention the serverInfo tool.
    await expect(dialog).toContainText("serverInfo");

    // Click Allow.
    const allowBtn = page.getByTestId("permission-allow");
    await expect(allowBtn).toBeVisible();
    await allowBtn.click();

    // The agent should receive the serverInfo result and mention "carve-agentos" in its response.
    const msgs = page.locator(".copilotKitAssistantMessage");
    await expect(msgs.last()).toBeVisible({ timeout: 45_000 });
    await expect(page.locator(".copilotKitMessages")).toContainText(
      "carve-agentos",
      { timeout: 15_000 },
    );
  });

  test.skip("permission denial blocks backend tool execution", async ({
    page,
  }) => {
    await sendChatMessage(
      page,
      "Use the serverInfo tool to get server information.",
    );

    // Wait for the permission dialog to appear.
    const dialog = page.getByTestId("permission-dialog");
    await expect(dialog).toBeVisible({ timeout: 45_000 });

    // Click Deny.
    const denyBtn = page.getByTestId("permission-deny");
    await expect(denyBtn).toBeVisible();
    await denyBtn.click();

    // The agent should acknowledge the denial in some way (message about denied/blocked).
    const msgs = page.locator(".copilotKitAssistantMessage");
    await expect(msgs.last()).toBeVisible({ timeout: 45_000 });

    // The response should NOT contain the tool's success payload.
    await expect(page.locator(".copilotKitMessages")).not.toContainText(
      "carve-agentos",
      { timeout: 5_000 },
    );
  });

  test.skip("handles tool execution error gracefully", async ({ page }) => {
    await sendChatMessage(
      page,
      "Trigger an error by using the failingTool.",
    );

    // Permission plugin will first ask for approval.
    const dialog = page.getByTestId("permission-dialog");
    await expect(dialog).toBeVisible({ timeout: 45_000 });

    // Approve so the tool actually executes and fails.
    const allowBtn = page.getByTestId("permission-allow");
    await expect(allowBtn).toBeVisible();
    await allowBtn.click();

    // The agent should respond acknowledging the error.
    const msgs = page.locator(".copilotKitAssistantMessage");
    await expect(msgs.last()).toBeVisible({ timeout: 45_000 });

    // The response should mention the failure.
    await expect(page.locator(".copilotKitMessages")).toContainText(/fail|error/i, {
      timeout: 15_000,
    });
  });

  test("HITL: button state feedback after approval", async ({ page }) => {
    await sendChatMessage(page, 'Delete the task "Write tests" from the list.');

    const approval = page.getByTestId("delete-approval");
    const approveBtn = approval.getByRole("button", { name: "Approve" });
    await expect(approveBtn).toBeVisible({ timeout: 45_000 });
    await expect(approveBtn).toBeEnabled();

    await approveBtn.click();

    // After approval: "Action completed" text should appear OR buttons should disappear.
    await expect(
      approval
        .getByText(/action completed/i)
        .or(approveBtn.locator("visible=false")),
    )
      .toBeVisible({ timeout: 10_000 })
      .catch(async () => {
        // Fallback: buttons may still be visible but disabled.
        if (await approveBtn.isVisible()) {
          await expect(approveBtn).toBeDisabled();
        }
      });
  });
});
