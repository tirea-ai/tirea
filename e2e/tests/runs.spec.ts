import { test, expect } from '@playwright/test';

test('start run with mock LLM returns SSE stream', async ({ request }) => {
  // First create a thread
  const threadRes = await request.post('/v1/threads', {
    data: { title: 'Run Test' },
  });
  const thread = await threadRes.json();

  // Start a run
  const runRes = await request.post('/v1/runs', {
    data: {
      agentId: 'default',
      threadId: thread.id,
      messages: [{ role: 'user', content: 'Hello' }],
    },
  });
  expect(runRes.ok()).toBeTruthy();
  expect(runRes.headers()['content-type']).toContain('text/event-stream');

  // Read some SSE data
  const body = await runRes.text();
  expect(body).toContain('data:');
});

test('start run without messages returns 400', async ({ request }) => {
  const response = await request.post('/v1/runs', {
    data: {
      agentId: 'default',
      messages: [],
    },
  });
  expect(response.status()).toBe(400);
});

test('start run without agentId returns 400', async ({ request }) => {
  const response = await request.post('/v1/runs', {
    data: {
      agentId: '',
      messages: [{ role: 'user', content: 'Hi' }],
    },
  });
  expect(response.status()).toBe(400);
});
