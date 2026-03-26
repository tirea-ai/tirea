import { test, expect } from '@playwright/test';

test('AI SDK chat endpoint returns SSE stream', async ({ request }) => {
  const response = await request.post('/v1/ai-sdk/chat', {
    data: {
      agentId: 'default',
      messages: [{ role: 'user', content: 'What is the weather?' }],
    },
  });
  expect(response.ok()).toBeTruthy();
  const body = await response.text();
  expect(body.length).toBeGreaterThan(0);
});
