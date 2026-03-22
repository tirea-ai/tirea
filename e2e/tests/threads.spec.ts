import { test, expect } from '@playwright/test';

test('create thread returns thread with id', async ({ request }) => {
  const response = await request.post('/v1/threads', {
    data: { title: 'Test Thread' },
  });
  expect(response.ok()).toBeTruthy();
  const body = await response.json();
  expect(body.id).toBeTruthy();
  expect(body.metadata.title).toBe('Test Thread');
});

test('list threads returns paginated items', async ({ request }) => {
  const response = await request.get('/v1/threads');
  expect(response.ok()).toBeTruthy();
  const body = await response.json();
  expect(Array.isArray(body.items)).toBeTruthy();
  expect(typeof body.offset).toBe('number');
  expect(typeof body.limit).toBe('number');
});

test('get nonexistent thread returns 404', async ({ request }) => {
  const response = await request.get('/v1/threads/nonexistent-id');
  expect(response.status()).toBe(404);
});
