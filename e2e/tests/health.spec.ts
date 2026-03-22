import { test, expect } from '@playwright/test';

test('health check returns 200', async ({ request }) => {
  const response = await request.get('/health');
  expect(response.ok()).toBeTruthy();
});
