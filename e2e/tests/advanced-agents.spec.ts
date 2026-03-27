import { test, expect } from '@playwright/test';

test.describe('advanced agent variants', () => {
  // Profile agent
  test('profile agent accepts run', async ({ request }) => {
    const res = await request.post('/v1/runs', {
      data: {
        agentId: 'profile',
        messages: [{ role: 'user', content: 'Remember my name is Alice' }],
      },
    });
    expect(res.ok()).toBeTruthy();
    const body = await res.text();
    expect(body).toContain('data:');
  });

  // Creative agent (custom context policy)
  test('creative agent accepts run', async ({ request }) => {
    const res = await request.post('/v1/runs', {
      data: {
        agentId: 'creative',
        messages: [{ role: 'user', content: 'Write a poem' }],
      },
    });
    expect(res.ok()).toBeTruthy();
  });

  // Compact agent (auto-compaction)
  test('compact agent accepts run', async ({ request }) => {
    const res = await request.post('/v1/runs', {
      data: {
        agentId: 'compact',
        messages: [{ role: 'user', content: 'Summarize a long document' }],
      },
    });
    expect(res.ok()).toBeTruthy();
  });

  // Budget agent (stop policies)
  test('budget agent terminates within limits', async ({ request }) => {
    const start = Date.now();
    const res = await request.post('/v1/runs', {
      data: {
        agentId: 'budget',
        messages: [{ role: 'user', content: 'Hello budget agent' }],
      },
    });
    expect(res.ok()).toBeTruthy();
    const elapsed = Date.now() - start;
    // Should terminate within 60s timeout
    expect(elapsed).toBeLessThan(60_000);
  });

  // Secured agent (permission rules)
  test('secured agent accepts run', async ({ request }) => {
    const res = await request.post('/v1/runs', {
      data: {
        agentId: 'secured',
        messages: [{ role: 'user', content: 'What is the weather?' }],
      },
    });
    expect(res.ok()).toBeTruthy();
  });

  // All new agents via AG-UI
  for (const agentId of ['profile', 'creative', 'compact', 'budget', 'secured']) {
    test(`${agentId} agent via AG-UI protocol`, async ({ request }) => {
      const res = await request.post('/v1/ag-ui/run', {
        data: {
          agentId,
          messages: [{ role: 'user', content: `Test ${agentId}` }],
        },
      });
      expect(res.ok()).toBeTruthy();
    });
  }

  // All new agents via AI SDK
  for (const agentId of ['profile', 'creative', 'compact', 'budget', 'secured']) {
    test(`${agentId} agent via AI SDK protocol`, async ({ request }) => {
      const res = await request.post('/v1/ai-sdk/chat', {
        data: {
          agentId,
          messages: [{ role: 'user', content: `Test ${agentId}` }],
        },
      });
      expect(res.ok()).toBeTruthy();
    });
  }

  // New agents in A2A agent list
  test('new agents appear in A2A list', async ({ request }) => {
    const res = await request.get('/v1/a2a/agents');
    if (res.ok()) {
      const agents = await res.json();
      if (Array.isArray(agents)) {
        const ids = agents.map((a: any) => a.agentId || a.id);
        for (const expected of ['profile', 'creative', 'compact', 'budget', 'secured']) {
          expect(ids).toContain(expected);
        }
      }
    }
  });
});

test.describe('enhanced features', () => {
  // Skills agent with embedded skill
  test('skills agent works with embedded skills', async ({ request }) => {
    const res = await request.post('/v1/runs', {
      data: {
        agentId: 'skills',
        messages: [{ role: 'user', content: 'Hello skills agent' }],
      },
    });
    expect(res.ok()).toBeTruthy();
  });

  // Reminder rules trigger on weather tool
  test('weather tool produces response (reminder may inject)', async ({ request }) => {
    const res = await request.post('/v1/runs', {
      data: {
        agentId: 'default',
        messages: [{ role: 'user', content: 'RUN_WEATHER_TOOL' }],
      },
    });
    expect(res.ok()).toBeTruthy();
    const body = await res.text();
    expect(body.length).toBeGreaterThan(0);
  });
});
