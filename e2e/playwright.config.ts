import { defineConfig } from '@playwright/test';

export default defineConfig({
  testDir: './tests',
  timeout: 30000,
  retries: 0,
  use: {
    baseURL: 'http://127.0.0.1:3000',
  },
  webServer: {
    command: 'cargo run --example starter_backend',
    cwd: '..',
    port: 3000,
    timeout: 120000,
    reuseExistingServer: true,
  },
});
