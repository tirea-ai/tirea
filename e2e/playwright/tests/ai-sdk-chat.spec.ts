import { test, expect } from "@playwright/test";

test("AI SDK chat: send message and receive streaming response", async ({
  page,
}) => {
  await page.goto("/");

  // Verify page loaded.
  await expect(page.locator("h1")).toHaveText("Uncarve Chat");

  // Type a message and submit.
  const input = page.getByPlaceholder("Type a message...");
  await input.fill("What is 2+2? Reply with just the number.");
  await page.getByRole("button", { name: "Send" }).click();

  // Wait for the assistant response to appear (contains "Agent:").
  const agentMsg = page.locator("strong", { hasText: "Agent:" }).first();
  await expect(agentMsg).toBeVisible({ timeout: 45_000 });

  // The response text should contain "4".
  const responseDiv = agentMsg.locator("..");
  await expect(responseDiv).toContainText("4", { timeout: 10_000 });
});
