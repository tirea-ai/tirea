import { expect, test } from "@playwright/test";

test("home page smoke flow", async ({ page }) => {
  await page.goto("/");

  await expect(page.getByTestId("page-title")).toHaveText("with-tirea canvas");
  await expect(page.getByTestId("persisted-threads-link")).toHaveAttribute(
    "href",
    "/persisted-threads",
  );
  await expect(page.getByTestId("canvas-link")).toHaveAttribute("href", "/");
  await expect(page.getByTestId("base-starter-link")).toHaveAttribute("href", "/v1");
  await expect(page.getByTestId("canvas-prompt")).toContainText("Prompt:");
  await expect(page.getByTestId("canvas-agent-running")).toContainText("Agent running:");
  await expect(page.getByTestId("shared-state-prompt")).toContainText(
    "Prompt: \"Add two todos about AG-UI contract verification.\"",
  );
  await expect(page.getByTestId("shared-state-json")).toContainText("\"todos\"");

  const pendingCount = page.getByTestId("canvas-pending-count");
  await expect(pendingCount).toContainText("Pending:");

  const initialText = await pendingCount.textContent();
  const initialCount = Number.parseInt(
    initialText?.replace("Pending:", "").trim() ?? "0",
    10,
  );

  await page.getByTestId("canvas-add-todo").click();
  await expect(pendingCount).toHaveText(`Pending: ${initialCount + 1}`);

  const firstTitle = page.getByTestId("canvas-title-input").first();
  await firstTitle.fill("Plan launch checklist");
  await expect(firstTitle).toHaveValue("Plan launch checklist");

  await page.getByTestId("canvas-toggle-status").first().click();
  await expect(page.getByTestId("canvas-completed-count")).toContainText("Completed: 1");
});
