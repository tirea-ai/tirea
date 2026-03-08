import { expect, test, type Page } from "@playwright/test";

async function openChat(page: Page, agentId = "default") {
  await page.goto(`/?agentId=${agentId}`);
  await expect(page.getByTestId("playground-root")).toBeVisible({
    timeout: 30_000,
  });
  await expect(page.getByTestId("chat-input")).toBeVisible({
    timeout: 30_000,
  });
}

async function sendPrompt(page: Page, prompt: string) {
  await page.getByTestId("chat-input").fill(prompt);
  await page.getByRole("button", { name: "Send" }).click();
}

async function waitForRunComplete(page: Page) {
  await page.waitForTimeout(500);
  await expect(page.getByRole("button", { name: "Send" })).toBeEnabled({
    timeout: 120_000,
  });
}

test.describe("A2UI Tool Integration", () => {
  test("render_a2ui tool renders surface with components", async ({ page }) => {
    await openChat(page, "a2ui");
    await sendPrompt(
      page,
      "RUN_A2UI_TOOL: render a demo surface with a greeting card",
    );
    await waitForRunComplete(page);

    // Verify the A2UI surface is rendered (not just the tool card)
    const surface = page.getByTestId("a2ui-surface");
    await expect(surface).toBeVisible({ timeout: 60_000 });

    // Verify tool card is also present
    await expect(page.getByTestId("tool-card").first()).toBeVisible({ timeout: 10_000 });

    await page.screenshot({ path: "/tmp/a2ui-test-surface.png", fullPage: true });
  });

  test("interactive form: fill field and submit triggers agent response", async ({
    page,
  }) => {
    await openChat(page, "a2ui");

    // Ask for a form with a text field and submit button
    await sendPrompt(
      page,
      'Use render_a2ui to create a contact form surface (surfaceId "contact") with: ' +
        'a Card root, a Column, a TextField (id "name_field", label "Name", value bound to "/name"), ' +
        'and a Button (id "submit_btn", text "Submit", action event name "submit"). ' +
        'Set initial data model at "/" with {"name": ""}.',
    );
    await waitForRunComplete(page);

    // Wait for the A2UI surface to render
    const surface = page.getByTestId("a2ui-surface");
    await expect(surface).toBeVisible({ timeout: 60_000 });

    await page.screenshot({ path: "/tmp/a2ui-test-form.png", fullPage: true });

    // Fill in the text field
    const nameInput = page.getByTestId("a2ui-input-name_field");
    await expect(nameInput).toBeVisible({ timeout: 10_000 });
    await nameInput.fill("Alice");

    // Click the submit button
    const submitBtn = page.getByTestId("a2ui-button-submit_btn");
    await expect(submitBtn).toBeVisible({ timeout: 10_000 });
    await submitBtn.click();

    // The button click should trigger a new message to the agent
    // Wait for the agent to respond to the A2UI event
    await waitForRunComplete(page);

    // Verify the agent responded (the event message and agent's response should be visible)
    const main = page.locator("main");
    await expect(main).toContainText("A2UI event", { timeout: 60_000 });

    await page.screenshot({
      path: "/tmp/a2ui-test-interaction.png",
      fullPage: true,
    });
  });
});
