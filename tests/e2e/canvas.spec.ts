import { expect, test } from "@playwright/test";

test("v1 compatibility page smoke flow", async ({ page }) => {
  await page.goto("/v1");

  await expect(page.getByTestId("page-title")).toHaveText("with-tirea");
  await expect(page.getByTestId("base-starter-link")).toHaveAttribute("href", "/v1");
  await expect(page.getByTestId("canvas-link")).toHaveAttribute("href", "/");
  await expect(page.getByTestId("persisted-threads-link")).toHaveAttribute(
    "href",
    "/persisted-threads",
  );
  await expect(page.getByTestId("shared-state-prompt")).toContainText(
    "Prompt: \"Add a todo: verify AG-UI event ordering.\"",
  );
  await expect(page.getByTestId("shared-state-json")).toContainText("\"todos\"");
  await expect(page.getByTestId("frontend-actions-prompt")).toContainText(
    "Prompt: \"Set theme color to #16a34a, then add todo: ship starter.\"",
  );
  await expect(page.getByTestId("generative-ui-prompt")).toContainText(
    "Prompt: \"Call showReleaseChecklist with items: run tests, tag release.\"",
  );
  await expect(page.getByTestId("generative-ui-preview")).toContainText(
    "Expected result: a checklist card is rendered inside Copilot chat.",
  );
  await expect(page.getByTestId("backend-tool-prompt")).toContainText(
    "Optional backend-tool prompt:",
  );
  await expect(page.getByTestId("hitl-prompt")).toContainText(
    "Prompt: \"Ask me to approve clearing todos before executing.\"",
  );
  await expect(page.getByTestId("hitl-status")).toHaveText("Approval status: pending");

  const todoCount = page.getByTestId("todo-count");
  await expect(todoCount).toContainText("Count:");

  await page.getByTestId("local-todo-input").fill("Validate basic Playwright check");
  await page.getByTestId("add-local-todo").click();

  await expect(page.getByTestId("local-todo-input")).toHaveValue("");
  await expect(todoCount).toContainText("Count:");

  await page.getByTestId("theme-green").click();
  await expect(page.getByTestId("frontend-action-last")).toHaveText(
    "Last action: localSetTheme(#16a34a)",
  );
  await expect(page.getByTestId("home-root")).toHaveCSS(
    "background-color",
    "rgb(22, 163, 74)",
  );

  await page.getByTestId("clear-local-todos").click();
  await expect(todoCount).toHaveText("Count: 0");

  await page.getByTestId("open-hitl-preview").click();
  await page.getByTestId("hitl-deny").click();
  await expect(page.getByTestId("hitl-status")).toHaveText("Approval status: denied");
});
