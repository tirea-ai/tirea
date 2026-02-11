import { defineConfig } from "@playwright/test";

const AI_SDK_PORT = process.env.AI_SDK_PORT ?? "3001";
const COPILOTKIT_PORT = process.env.COPILOTKIT_PORT ?? "3002";

export default defineConfig({
  testDir: "./tests",
  timeout: 60_000,
  retries: 1,
  reporter: [["list"]],
  use: {
    headless: true,
  },
  projects: [
    {
      name: "ai-sdk",
      testMatch: "ai-sdk-chat.spec.ts",
      use: { baseURL: `http://localhost:${AI_SDK_PORT}` },
    },
    {
      name: "copilotkit",
      testMatch: "copilotkit-chat.spec.ts",
      use: { baseURL: `http://localhost:${COPILOTKIT_PORT}` },
    },
  ],
});
