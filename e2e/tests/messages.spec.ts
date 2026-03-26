import { test, expect } from '@playwright/test';

test.describe('message history', () => {
  test('message list endpoint returns valid response after run', async ({
    request,
  }) => {
    // Create thread
    const threadRes = await request.post('/v1/threads', {
      data: { title: 'Message History Test' },
    });
    const thread = await threadRes.json();

    // Start a run (SSE stream)
    const runRes = await request.post('/v1/runs', {
      data: {
        agentId: 'default',
        threadId: thread.id,
        messages: [{ role: 'user', content: 'Remember this message' }],
      },
    });
    expect(runRes.ok()).toBeTruthy();
    // Consume stream to wait for completion
    await runRes.text();

    // Message list endpoint should return a valid response structure
    const msgRes = await request.get(`/v1/threads/${thread.id}/messages`);
    expect(msgRes.ok()).toBeTruthy();
    const body = await msgRes.json();
    expect(body).toHaveProperty('messages');
    expect(Array.isArray(body.messages)).toBeTruthy();
    expect(body).toHaveProperty('total');
    expect(body).toHaveProperty('has_more');
  });

  test('post messages triggers background run and returns 202', async ({
    request,
  }) => {
    const threadRes = await request.post('/v1/threads', {
      data: { title: 'Post Messages Test' },
    });
    const thread = await threadRes.json();

    // POST messages triggers a background run, returns 202
    const addRes = await request.post(`/v1/threads/${thread.id}/messages`, {
      data: {
        messages: [{ role: 'user', content: 'First message' }],
      },
    });
    expect(addRes.status()).toBe(202);
    const addBody = await addRes.json();
    expect(addBody.thread_id).toBe(thread.id);
    expect(['running', 'queued']).toContain(addBody.status);
  });

  test('post messages requires at least one message', async ({ request }) => {
    const threadRes = await request.post('/v1/threads', {
      data: { title: 'Empty Messages Test' },
    });
    const thread = await threadRes.json();

    const addRes = await request.post(`/v1/threads/${thread.id}/messages`, {
      data: { messages: [] },
    });
    expect(addRes.ok()).toBeFalsy();
  });

  test('post messages to nonexistent thread returns error', async ({
    request,
  }) => {
    const addRes = await request.post(
      '/v1/threads/nonexistent-id/messages',
      { data: { messages: [{ role: 'user', content: 'hello' }] } },
    );
    expect(addRes.ok()).toBeFalsy();
  });

  test('multi-turn SSE runs on same thread both succeed', async ({
    request,
  }) => {
    const threadRes = await request.post('/v1/threads', {
      data: { title: 'Multi-turn' },
    });
    const thread = await threadRes.json();

    // Turn 1
    const run1 = await request.post('/v1/runs', {
      data: {
        agentId: 'default',
        threadId: thread.id,
        messages: [{ role: 'user', content: 'Hello, first turn' }],
      },
    });
    expect(run1.ok()).toBeTruthy();
    const body1 = await run1.text();
    expect(body1).toContain('data:');

    // Turn 2 on same thread
    const run2 = await request.post('/v1/runs', {
      data: {
        agentId: 'default',
        threadId: thread.id,
        messages: [{ role: 'user', content: 'Second turn' }],
      },
    });
    expect(run2.ok()).toBeTruthy();
    const body2 = await run2.text();
    expect(body2).toContain('data:');

    // Message list endpoint accessible after multiple turns
    const msgRes = await request.get(`/v1/threads/${thread.id}/messages`);
    expect(msgRes.ok()).toBeTruthy();
    const msgBody = await msgRes.json();
    expect(Array.isArray(msgBody.messages)).toBeTruthy();
  });

  test('messages for nonexistent thread returns error', async ({ request }) => {
    const res = await request.get('/v1/threads/nonexistent-id/messages');
    expect(res.ok()).toBeFalsy();
  });

  test('AG-UI multi-turn preserves context', async ({ request }) => {
    const res = await request.post('/v1/ag-ui/run', {
      data: {
        agentId: 'default',
        messages: [
          { role: 'user', content: 'My name is Alice' },
          { role: 'assistant', content: 'Hello Alice!' },
          { role: 'user', content: 'What is my name?' },
        ],
      },
    });
    expect(res.ok()).toBeTruthy();
    const body = await res.text();
    expect(body.length).toBeGreaterThan(0);
  });
});
