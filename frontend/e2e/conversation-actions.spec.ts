// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: E2E tests for the conversation row's own actions — rename, mark unread, delete
// ABOUTME: Web equivalents of the mobile swipe gestures, on the unified list's rows

import { test, expect } from '@playwright/test';
import { setupDashboardMocks, loginToDashboard } from './test-helpers';

const mockConversations = {
  conversations: [
    {
      id: 'conv-1',
      title: 'Training Plan Discussion',
      coach_id: 'coach-marathon',
      coach_title: 'Marathon Coach',
      coach_handle: 'marathon-coach',
      created_at: '2024-06-01T10:00:00Z',
      updated_at: '2024-06-01T12:00:00Z',
      message_count: 5,
      unread_count: 0,
      last_message: {
        preview: 'Hold the long run at 2h30.',
        role: 'assistant',
        created_at: '2024-06-01T12:00:00Z',
      },
    },
    {
      id: 'conv-2',
      title: 'Nutrition Questions',
      coach_id: 'coach-nutrition',
      coach_title: 'Nutrition Coach',
      coach_handle: 'nutrition-coach',
      created_at: '2024-05-28T08:00:00Z',
      updated_at: '2024-05-30T09:00:00Z',
      message_count: 3,
      unread_count: 2,
      last_message: {
        preview: 'Try 60g of carbs an hour.',
        role: 'assistant',
        created_at: '2024-05-30T09:00:00Z',
      },
    },
  ],
  total: 2,
  limit: 50,
  offset: 0,
};

async function setupConversationMocks(page: import('@playwright/test').Page) {
  // Override the default empty conversations mock from setupDashboardMocks
  await page.route('**/api/chat/conversations**', async (route, request) => {
    if (request.method() === 'GET') {
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify(mockConversations),
      });
    } else if (request.method() === 'DELETE') {
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({ success: true }),
      });
    } else if (request.method() === 'PATCH') {
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({
          ...mockConversations.conversations[0],
          title: 'Renamed Conversation',
        }),
      });
    } else {
      await route.fallback();
    }
  });
}

test.describe('Conversation Management Actions', () => {
  test.beforeEach(async ({ page }) => {
    // Catch-all: forward unmocked API/OAuth requests to specific route handlers via fallback().
    // Uses URL function (not glob) to avoid intercepting Vite source file paths like /src/services/api/client.ts.
    // Registered first so it runs last in LIFO; fallback() ensures specific mocks take priority.
    await page.route((url) => url.pathname.startsWith('/api/') || url.pathname.startsWith('/oauth/'), async (route) => {
      await route.fallback();
    });
    await setupDashboardMocks(page, { role: 'user' });
    await setupConversationMocks(page);
    // Mock endpoints needed by ChatTab (user's default tab) to prevent 401 logout
    await page.route('**/api/providers', async (route) => {
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({ providers: [] }),
      });
    });
    await page.route('**/api/coaches**', async (route) => {
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({ coaches: [], total: 0 }),
      });
    });
    await loginToDashboard(page);
  });

  test('should display conversations in sidebar', async ({ page }) => {
    // User defaults to Chat tab, conversations should be in sidebar
    await expect(page.getByText('Training Plan Discussion')).toBeVisible({ timeout: 10000 });
    await expect(page.getByText('Nutrition Questions')).toBeVisible();
  });

  test('should reveal the row actions menu on hover', async ({ page }) => {
    await expect(page.getByText('Training Plan Discussion')).toBeVisible({ timeout: 10000 });

    // Hover over the row to reveal its single actions trigger (group-hover)
    const row = page.locator('[data-testid="conversation-row"]', {
      hasText: 'Training Plan Discussion',
    });
    await row.hover();

    await row.getByTestId('conversation-actions-trigger').click();
    const menu = row.getByRole('menu', { name: 'Conversation actions' });
    await expect(menu.getByRole('menuitem')).toHaveText([
      'Rename conversation',
      'Mark conversation unread',
      'Delete conversation',
    ]);
  });

  test('the row stays clickable while its actions are showing', async ({ page }) => {
    await expect(page.getByText('Training Plan Discussion')).toBeVisible({ timeout: 10000 });

    // Hovering used to lay three buttons over the right half of the row, so
    // clicking there stopped opening the thread.
    const row = page.locator('[data-testid="conversation-row"]', {
      hasText: 'Training Plan Discussion',
    });
    await row.hover();
    await row.getByRole('button', { name: /Training Plan Discussion/ }).click();

    await expect(page).toHaveURL(/#chat\/conv-1$/);
  });

  test('should enable rename mode from the row actions menu', async ({ page }) => {
    await expect(page.getByText('Training Plan Discussion')).toBeVisible({ timeout: 10000 });

    const row = page.locator('[data-testid="conversation-row"]', {
      hasText: 'Training Plan Discussion',
    });
    await row.hover();
    await row.getByTestId('conversation-actions-trigger').click();
    await row.getByRole('menuitem', { name: 'Rename conversation' }).click();

    // Input field should appear with the current title
    const input = page.getByLabel('Conversation title');
    await expect(input).toBeVisible();
    await expect(input).toHaveValue('Training Plan Discussion');
  });

  test('shows the unread count on a row with unread messages, and clears it on demand', async ({ page }) => {
    const cleared: string[] = [];
    await page.route('**/api/chat/conversations/*/read', async (route, request) => {
      if (request.method() === 'DELETE') cleared.push(request.url());
      await route.fulfill({ status: 204, body: '' });
    });

    const unreadRow = page.locator('[data-testid="conversation-row"]', {
      hasText: 'Nutrition Questions',
    });
    await expect(unreadRow.getByTestId('conversation-unread-count')).toHaveText('2', {
      timeout: 10000,
    });

    const readRow = page.locator('[data-testid="conversation-row"]', {
      hasText: 'Training Plan Discussion',
    });
    await expect(readRow.getByTestId('conversation-unread-count')).toHaveCount(0);

    await readRow.hover();
    await readRow.getByTestId('conversation-actions-trigger').click();
    await readRow.getByRole('menuitem', { name: 'Mark conversation unread' }).click();
    await expect.poll(() => cleared.length).toBe(1);
    expect(cleared[0]).toContain('/api/chat/conversations/conv-1/read');
  });

  test('shows the last-message preview beside each row', async ({ page }) => {
    const row = page.locator('[data-testid="conversation-row"]', {
      hasText: 'Training Plan Discussion',
    });
    await expect(row.getByTestId('conversation-preview')).toHaveText('Hold the long run at 2h30.', {
      timeout: 10000,
    });
  });
});
