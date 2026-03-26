import { test, expect } from '@playwright/test';

test('AG-UI run endpoint returns SSE stream', async ({ request }) => {
  const response = await request.post('/v1/ag-ui/run', {
    data: {
      agentId: 'default',
      messages: [{ role: 'user', content: 'Hello agent' }],
    },
  });
  expect(response.ok()).toBeTruthy();
  const body = await response.text();
  expect(body.length).toBeGreaterThan(0);
});
