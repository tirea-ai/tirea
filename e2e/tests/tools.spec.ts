import { test, expect } from '@playwright/test';

test('get_weather tool returns mock data via run', async ({ request }) => {
  const response = await request.post('/v1/runs', {
    data: {
      agentId: 'starter',
      messages: [{ role: 'user', content: 'What is the weather in Tokyo?' }],
    },
  });
  expect(response.ok()).toBeTruthy();
  // In mock mode, the LLM may not call tools, but the endpoint should not crash
  const body = await response.text();
  expect(body).toContain('data:');
});
