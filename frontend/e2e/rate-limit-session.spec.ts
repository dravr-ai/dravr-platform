// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: E2E tests for a spent request budget: a 429 keeps the athlete signed in and reads as the usage quota
// ABOUTME: Session restore answering 429 must not bounce to login; a 429 on new chat must not claim the conversation cap

import { test, expect, type Page } from '@playwright/test';
import { setupDashboardMocks, loginToDashboard, openChat, APP_SHELL_TIMEOUT_MS } from './test-helpers';

/** The refusal the server answers once the monthly request budget is spent. */
const BUDGET_SPENT = {
  code: 'RateLimitExceeded',
  message: 'Rate limit exceeded: 10000/10000 requests, retry after 3600s',
  details: { limit_type: 'requests', current: 10000, limit: 10000, retry_after_secs: 3600 },
  timestamp: '2026-09-25T12:00:00Z',
};

const CONVERSATION = {
  id: 'conv-1',
  title: 'Sunday long run',
  agent_id: null,
  created_at: '2026-09-20T10:00:00Z',
  updated_at: '2026-09-20T10:05:00Z',
  message_count: 0,
  unread_count: 0,
};

async function fulfillBudgetSpent(route: import('@playwright/test').Route) {
  await route.fulfill({
    status: 429,
    contentType: 'application/json',
    headers: {
      'retry-after': '3600',
      'x-ratelimit-limit': '10000',
      'x-ratelimit-remaining': '0',
    },
    body: JSON.stringify(BUDGET_SPENT),
  });
}

async function setupSpentBudgetMocks(page: Page) {
  // Base dashboard mocks first: later routes take priority (LIFO).
  await setupDashboardMocks(page, { role: 'user' });

  // The budget is spent: restoring the session answers 429, not 401.
  await page.route('**/api/auth/session', fulfillBudgetSpent);

  await page.route(/\/api\/chat\/conversations(\?.*)?$/, async (route, request) => {
    if (request.method() === 'POST') {
      await fulfillBudgetSpent(route);
      return;
    }
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({ conversations: [CONVERSATION], total: 1, limit: 50, offset: 0 }),
    });
  });
}

test.describe('Spent request budget', () => {
  test('a session restore answering 429 keeps the athlete signed in', async ({ page }) => {
    await setupSpentBudgetMocks(page);
    await loginToDashboard(page);

    const restore = page.waitForResponse('**/api/auth/session');
    await page.reload();
    expect((await restore).status()).toBe(429);

    // Still in the app shell, never bounced to the sign-in form.
    await expect(page.locator('main')).toBeVisible({ timeout: APP_SHELL_TIMEOUT_MS });
    await expect(page.locator('input[name="password"]')).toHaveCount(0);
    await expect(page).not.toHaveURL(/login/);
    expect(await page.evaluate(() => localStorage.getItem('pierre_user'))).not.toBeNull();
  });

  test('a new chat refused for the spent budget shows the usage quota, not the conversation cap', async ({
    page,
  }) => {
    await setupSpentBudgetMocks(page);
    await loginToDashboard(page);
    await openChat(page);

    await page.getByRole('button', { name: 'New', exact: true }).first().click();
    await page
      .getByRole('menu', { name: 'Start a conversation' })
      .getByRole('menuitem', { name: 'New chat' })
      .click();

    await expect(
      page.getByText('Usage quota reached (10000/10000). Please try again later.'),
    ).toBeVisible();
    await expect(page.getByText('Could not start chat')).toBeVisible();
    await expect(page.getByText('Conversation limit reached')).toHaveCount(0);
  });
});
