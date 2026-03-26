import { test, expect } from '@playwright/test';

test.describe('multi-agent variants', () => {
  test('default agent accepts run', async ({ request }) => {
    const res = await request.post('/v1/runs', {
      data: {
        agentId: 'default',
        messages: [{ role: 'user', content: 'Hello' }],
      },
    });
    expect(res.ok()).toBeTruthy();
    expect(res.headers()['content-type']).toContain('text/event-stream');
  });

  test('travel agent accepts run', async ({ request }) => {
    const res = await request.post('/v1/runs', {
      data: {
        agentId: 'travel',
        messages: [{ role: 'user', content: 'Plan a trip to Paris' }],
      },
    });
    expect(res.ok()).toBeTruthy();
    const body = await res.text();
    expect(body).toContain('data:');
  });

  test('research agent accepts run', async ({ request }) => {
    const res = await request.post('/v1/runs', {
      data: {
        agentId: 'research',
        messages: [{ role: 'user', content: 'Research quantum computing' }],
      },
    });
    expect(res.ok()).toBeTruthy();
    const body = await res.text();
    expect(body).toContain('data:');
  });

  test('permission agent accepts run', async ({ request }) => {
    const res = await request.post('/v1/runs', {
      data: {
        agentId: 'permission',
        messages: [{ role: 'user', content: 'Hello' }],
      },
    });
    expect(res.ok()).toBeTruthy();
  });

  test('stopper agent accepts run', async ({ request }) => {
    const res = await request.post('/v1/runs', {
      data: {
        agentId: 'stopper',
        messages: [{ role: 'user', content: 'Hello' }],
      },
    });
    expect(res.ok()).toBeTruthy();
  });

  test('skills agent accepts run', async ({ request }) => {
    const res = await request.post('/v1/runs', {
      data: {
        agentId: 'skills',
        messages: [{ role: 'user', content: 'Hello' }],
      },
    });
    expect(res.ok()).toBeTruthy();
  });

  test('a2a agent accepts run', async ({ request }) => {
    const res = await request.post('/v1/runs', {
      data: {
        agentId: 'a2a',
        messages: [{ role: 'user', content: 'Hello' }],
      },
    });
    expect(res.ok()).toBeTruthy();
  });

  test('nonexistent agent is rejected or falls back', async ({ request }) => {
    const res = await request.post('/v1/runs', {
      data: {
        agentId: 'nonexistent_agent_xyz',
        messages: [{ role: 'user', content: 'Hello' }],
      },
    });
    // Server may reject unknown agents (4xx) or fall back to default (200).
    // Either way the response should be well-formed.
    const status = res.status();
    expect(status).toBeGreaterThanOrEqual(200);
    expect(status).toBeLessThan(500);
  });

  test('AG-UI routes to travel agent', async ({ request }) => {
    const res = await request.post('/v1/ag-ui/run', {
      data: {
        agentId: 'travel',
        messages: [{ role: 'user', content: 'Find places in Tokyo' }],
      },
    });
    expect(res.ok()).toBeTruthy();
  });

  test('AI SDK routes to research agent', async ({ request }) => {
    const res = await request.post('/v1/ai-sdk/chat', {
      data: {
        agentId: 'research',
        messages: [{ role: 'user', content: 'Search for AI papers' }],
      },
    });
    expect(res.ok()).toBeTruthy();
  });
});
