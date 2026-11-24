// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: E2E for the composer's unsent line, kept per thread — opening another chat shows that chat's own draft
// ABOUTME: Types in one thread, opens another from History, and comes back: each composer holds only what was typed in it

import { test, expect, type Page } from '@playwright/test';
import { setupDashboardMocks, loginToDashboard, openPersonalConversation } from './test-helpers';

function conversation(id: string, title: string) {
  return {
    id,
    title,
    agent_id: null,
    created_at: '2026-11-20T10:00:00Z',
    updated_at: '2026-11-20T12:00:00Z',
    message_count: 2,
    unread_count: 0,
    last_message: { preview: 'Ready when you are.', role: 'assistant', created_at: '2026-11-20T12:00:00Z' },
  };
}

const THREADS = [conversation('conv-plan', 'Plan for the week'), conversation('conv-race', 'Race day questions')];

async function mockTwoThreads(page: Page) {
  await page.route('**/api/chat/conversations**', async (route, request) => {
    const path = new URL(request.url()).pathname;
    if (request.method() === 'GET' && path === '/api/chat/conversations') {
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({ conversations: THREADS, total: THREADS.length, limit: 50, offset: 0 }),
      });
      return;
    }
    if (request.method() === 'GET' && path.endsWith('/messages')) {
      const id = path.split('/')[4];
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({
          messages: [
            { id: `${id}-q`, role: 'user', content: 'Hi', created_at: '2026-11-20T11:59:00Z' },
            { id: `${id}-a`, role: 'assistant', content: 'Ready when you are.', created_at: '2026-11-20T12:00:00Z' },
          ],
          feedback: [],
        }),
      });
      return;
    }
    if (request.method() === 'GET' && path.endsWith('/verdicts')) {
      await route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify({ verdicts: [] }) });
      return;
    }
    await route.fallback();
  });
}

function composer(page: Page) {
  return page.locator('textarea[data-composer="true"]').filter({ visible: true }).first();
}

test.describe('Composer draft per thread', () => {
  test.beforeEach(async ({ page }) => {
    // Registered first so it runs last: specific mocks below take priority.
    await page.route((url) => url.pathname.startsWith('/api/') || url.pathname.startsWith('/oauth/'), async (route) => {
      await route.fallback();
    });
    await setupDashboardMocks(page, { role: 'user' });
    await mockTwoThreads(page);
    await page.route('**/api/agents**', async (route) => {
      await route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify({ agents: [], total: 0 }) });
    });
    await loginToDashboard(page);
  });

  test('each thread keeps the line typed in it, and only that line', async ({ page }) => {
    await openPersonalConversation(page, 'Plan for the week');
    await expect(page.getByText('Ready when you are.').first()).toBeVisible();
    await composer(page).fill('je suis pret');

    await openPersonalConversation(page, 'Race day questions');
    await expect(composer(page)).toHaveValue('');
    await composer(page).fill('What should I eat the night before?');

    await openPersonalConversation(page, 'Plan for the week');
    await expect(composer(page)).toHaveValue('je suis pret');

    await openPersonalConversation(page, 'Race day questions');
    await expect(composer(page)).toHaveValue('What should I eat the night before?');
  });
});
