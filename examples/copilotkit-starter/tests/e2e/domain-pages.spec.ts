import { expect, test } from "@playwright/test";

test("travel page smoke flow", async ({ page }) => {
  await page.goto("/travel");

  await expect(page.getByTestId("travel-page-title")).toHaveText("Travel Planner");
  await expect(page.getByTestId("travel-link")).toHaveAttribute("href", "/travel");
  await expect(page.getByTestId("research-link")).toHaveAttribute("href", "/research");
  await expect(page.getByTestId("canvas-link")).toHaveAttribute("href", "/");
});

test("research page smoke flow", async ({ page }) => {
  await page.goto("/research");

  await expect(page.getByTestId("research-page-title")).toHaveText("Research Assistant");
  await expect(page.getByTestId("research-link")).toHaveAttribute("href", "/research");
  await expect(page.getByTestId("travel-link")).toHaveAttribute("href", "/travel");
  await expect(page.getByTestId("persisted-threads-link")).toHaveAttribute(
    "href",
    "/persisted-threads",
  );
});
