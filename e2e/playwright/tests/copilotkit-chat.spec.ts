import { test, expect } from "@playwright/test";

test.describe("CopilotKit Starter (with-tirea)", () => {
  test("page loads and renders canvas", async ({ page }) => {
    await page.goto("/");
    await expect(page.getByTestId("page-title")).toBeVisible({ timeout: 15_000 });
    await expect(page.getByTestId("page-title")).toContainText("with-tirea");
  });

  test("chat panel is visible", async ({ page }) => {
    await page.goto("/");
    await expect(page.locator(".starter-chat-v2")).toBeVisible({ timeout: 15_000 });
  });

  test("basic chat: send message and receive response", async ({ page }) => {
    await page.goto("/");

    const input = page.locator(".starter-chat-v2 textarea, .starter-chat-v2 input[type='text']").first();
    await expect(input).toBeVisible({ timeout: 15_000 });
    await input.fill("What is 2+2? Reply with just the number.");

    const sendBtn = page.locator('.starter-chat-v2 button[aria-label="Send"]');
    await expect(sendBtn).toBeVisible({ timeout: 5_000 });
    await sendBtn.click();

    // Wait for any assistant message to appear.
    const assistantMsg = page.locator(".copilotKitAssistantMessage").first();
    await expect(assistantMsg).toBeVisible({ timeout: 45_000 });
    await expect(assistantMsg).toContainText("4", { timeout: 15_000 });
  });
});
