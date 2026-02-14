import { test, expect } from "@playwright/test";

test.describe("AI SDK Chat", () => {
  test("page renders with heading, input, and send button", async ({
    page,
  }) => {
    await page.goto("/");

    await expect(page.locator("h1")).toHaveText("Uncarve Chat", {
      timeout: 15_000,
    });

    // Input and button should be present.
    await expect(page.getByPlaceholder("Type a message...")).toBeVisible();
    await expect(page.getByRole("button", { name: "Send" })).toBeVisible();
  });

  test("send message and receive streaming response", async ({ page }) => {
    await page.goto("/");

    await expect(page.locator("h1")).toHaveText("Uncarve Chat", {
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

    // The response should contain "4".
    const responseDiv = agentMsg.locator("..");
    await expect(responseDiv).toContainText("4", { timeout: 10_000 });
  });

  test("multi-turn conversation preserves history", async ({ page }) => {
    await page.goto("/");

    await expect(page.locator("h1")).toHaveText("Uncarve Chat", {
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

    await expect(page.locator("h1")).toHaveText("Uncarve Chat", {
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

    await expect(page.locator("h1")).toHaveText("Uncarve Chat", {
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

  test("send button is disabled while loading", async ({ page }) => {
    await page.goto("/");

    await expect(page.locator("h1")).toHaveText("Uncarve Chat", {
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
    await expect(sendButton).toBeEnabled({ timeout: 5_000 });
  });
});
