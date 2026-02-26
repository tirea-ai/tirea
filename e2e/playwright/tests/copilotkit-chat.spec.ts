import { test, expect } from "@playwright/test";

test.describe("CopilotKit Starter (with-tirea)", () => {
  test("page loads and renders canvas", async ({ page }) => {
    await page.goto("/");
    await expect(page.getByTestId("page-title")).toBeVisible({ timeout: 15_000 });
    await expect(page.getByTestId("page-title")).toContainText("with-tirea");
  });

  test("chat panel is visible", async ({ page }) => {
    await page.goto("/");
    await expect(page.getByRole("complementary")).toBeVisible({ timeout: 15_000 });
  });

  test("basic chat: send message and receive response", async ({ page }) => {
    await page.goto("/");

    const input = page.getByPlaceholder("Type a message...");
    await expect(input).toBeVisible({ timeout: 15_000 });
    await input.fill("What is 2+2? Reply with just the number.");
    await input.press("Enter");

    // CopilotKit v2 renders assistant replies inside the complementary aside.
    // The "4" response appears as a paragraph in the chat message list.
    const chatPanel = page.getByRole("complementary");
    await expect(chatPanel).toContainText("4", { timeout: 45_000 });
  });
});
