import { expect, test } from "@playwright/test";

test("renders release checklist items for showReleaseChecklist prompt", async ({ page }) => {
  test.setTimeout(180_000);

  await page.goto("/v1");

  const openChatButton = page.getByRole("button", { name: "Open Chat" });
  if (await openChatButton.count()) {
    await openChatButton.first().click().catch(() => {});
  }

  const input = page.getByPlaceholder("Type a message...");
  await input.fill("Call showReleaseChecklist with items: run tests, tag release.");
  await input.press("Enter");

  const checklist = page.getByTestId("generative-ui-checklist");
  await expect(checklist).toBeVisible({ timeout: 120_000 });

  const checklistText = (await checklist.textContent()) ?? "";
  expect(checklistText).toContain("run tests");
  expect(checklistText).toContain("tag release");

  await expect(page.getByTestId("generative-ui-card-error")).toHaveCount(0);
});
