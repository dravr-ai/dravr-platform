// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: E2E tests for conversation management actions in the chat sidebar
// ABOUTME: Web equivalents of mobile swipe gestures (rename, delete)

import { test, expect } from '@playwright/test';
import { setupDashboardMocks, loginToDashboard } from './test-helpers';

const mockConversations = {
  conversations: [
    {
      id: 'conv-1',
      title: 'Training Plan Discussion',
      coach_id: 'coach-marathon',
      coach_name: 'Marathon Coach',
      created_at: '2024-06-01T10:00:00Z',
      updated_at: '2024-06-01T12:00:00Z',
      message_count: 5,
    },
    {
      id: 'conv-2',
      title: 'Nutrition Questions',
      coach_id: 'coach-nutrition',
      coach_name: 'Nutrition Coach',
      created_at: '2024-05-28T08:00:00Z',
      updated_at: '2024-05-30T09:00:00Z',
      message_count: 3,
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

  test('should show rename and delete buttons on hover', async ({ page }) => {
    await expect(page.getByText('Training Plan Discussion')).toBeVisible({ timeout: 10000 });

    // Hover over the conversation item to reveal action buttons (group-hover)
    const conversationItem = page.locator('button:has-text("Training Plan Discussion")');
    await conversationItem.hover();

    // Rename and delete buttons should appear (scoped to this conversation item)
    await expect(conversationItem.getByRole('button', { name: /Rename conversation/i })).toBeVisible();
    await expect(conversationItem.getByRole('button', { name: /Delete conversation/i })).toBeVisible();
  });

  test('should enable rename mode when clicking rename button', async ({ page }) => {
    await expect(page.getByText('Training Plan Discussion')).toBeVisible({ timeout: 10000 });

    // Hover over conversation item to reveal action buttons
    const conversationItem = page.locator('button:has-text("Training Plan Discussion")');
    await conversationItem.hover();

    // Click rename button (scoped to this conversation item)
    const renameButton = conversationItem.getByRole('button', { name: /Rename conversation/i });
    await renameButton.click();

    // Input field should appear with the current title
    const input = page.locator('input[type="text"]').first();
    await expect(input).toBeVisible();
    await expect(input).toHaveValue('Training Plan Discussion');
  });
});
