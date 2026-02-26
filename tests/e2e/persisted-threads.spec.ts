import { expect, test } from "@playwright/test";

test("persisted page wires explicit thread id", async ({ page }) => {
  await page.goto("/persisted-threads");

  await expect(page.getByTestId("page-title")).toHaveText("with-tirea persisted threads");
  await expect(page.getByTestId("thread-prompt")).toContainText("Prompt:");
  await expect(page.getByTestId("active-thread")).toContainText("Active thread:");
  await expect(page.getByTestId("thread-list")).toBeVisible();
  await expect(page.getByTestId("base-starter-link")).toHaveAttribute("href", "/v1");
  await expect(page.getByTestId("canvas-link")).toHaveAttribute("href", "/");

  const firstThread = await page.getByTestId("active-thread").textContent();

  await page.getByTestId("thread-create").click();
  const secondThread = await page.getByTestId("active-thread").textContent();
  expect(secondThread).not.toEqual(firstThread);

  await page.getByTestId("local-todo-input").fill("Persisted task");
  await page.getByTestId("add-local-todo").click();
  await expect(page.getByTestId("todo-count")).toHaveText("Count: 1");
  await expect(page.getByTestId("todo-list")).toContainText("Persisted task");

  await page.getByTestId("clear-local-todos").click();
  await expect(page.getByTestId("todo-count")).toHaveText("Count: 0");
});

test("loads thread list from backend route and supports switch", async ({ page }) => {
  await page.route("**/api/threads", async (route) => {
    await route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify(["thread-alpha", "thread-beta"]),
    });
  });

  await page.goto("/persisted-threads");

  await expect(page.getByTestId("thread-item-thread-alpha")).toBeVisible();
  await expect(page.getByTestId("thread-item-thread-beta")).toBeVisible();
  await expect(page.getByTestId("active-thread")).toContainText("thread-alpha");

  await page.getByTestId("thread-item-thread-beta").click();
  await expect(page.getByTestId("active-thread")).toContainText("thread-beta");
});

test("falls back to default thread when backend list fails", async ({ page }) => {
  await page.route("**/api/threads", async (route) => {
    await route.fulfill({
      status: 500,
      contentType: "application/json",
      body: "{}",
    });
  });

  await page.goto("/persisted-threads");

  await expect(page.getByTestId("thread-item-local-fallback")).toBeVisible();
  await expect(page.getByTestId("active-thread")).toHaveText("Active thread: local-fallback");
});

test("threads API route returns array payload", async ({ request }) => {
  const response = await request.get("/api/threads");
  expect(response.ok()).toBeTruthy();
  const body = await response.json();
  expect(Array.isArray(body)).toBeTruthy();
});
