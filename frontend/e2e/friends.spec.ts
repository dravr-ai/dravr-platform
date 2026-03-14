// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Playwright E2E tests for Friends Management features.
// ABOUTME: Tests friend list display, search, send/accept/decline requests, and removal.

import { test, expect, type Page } from '@playwright/test';
import { setupDashboardMocks, loginToDashboard, navigateToTab } from './test-helpers';

// Mock friends list response matching ListFriendsResponse
const mockFriends = {
  friends: [
    {
      id: 'conn-1',
      user_id: 'user-123',
      friend_user_id: 'user-2',
      friend_display_name: 'Alice Runner',
      friend_email: 'alice@example.com',
      status: 'accepted',
      created_at: '2024-05-01T10:00:00Z',
      accepted_at: '2024-05-01T10:30:00Z',
    },
    {
      id: 'conn-2',
      user_id: 'user-123',
      friend_user_id: 'user-3',
      friend_display_name: 'Bob Cyclist',
      friend_email: 'bob@example.com',
      status: 'accepted',
      created_at: '2024-04-15T08:00:00Z',
      accepted_at: '2024-04-15T08:15:00Z',
    },
  ],
  total: 2,
  metadata: { timestamp: '2024-06-01T10:00:00Z', api_version: 'v1' },
};

// Mock pending requests response matching PendingRequestsResponse
const mockPendingRequests = {
  received: [
    {
      id: 'req-1',
      user_id: 'user-4',
      friend_user_id: 'user-123',
      user_display_name: 'Carol Swimmer',
      user_email: 'carol@example.com',
      status: 'pending',
      created_at: '2024-06-01T09:00:00Z',
    },
  ],
  sent: [
    {
      id: 'req-2',
      user_id: 'user-123',
      friend_user_id: 'user-5',
      user_display_name: 'Dave Triathlete',
      user_email: 'dave@example.com',
      status: 'pending',
      created_at: '2024-06-01T07:00:00Z',
    },
  ],
  metadata: { timestamp: '2024-06-01T10:00:00Z', api_version: 'v1' },
};

// Mock search results matching SearchUsersResponse
const mockSearchResults = {
  users: [
    {
      id: 'user-10',
      display_name: 'Eve Marathoner',
      email: 'eve@example.com',
      is_friend: false,
      has_pending_request: false,
    },
    {
      id: 'user-2',
      display_name: 'Alice Runner',
      email: 'alice@example.com',
      is_friend: true,
      has_pending_request: false,
    },
    {
      id: 'user-11',
      display_name: 'Frank Sprinter',
      email: 'frank@example.com',
      is_friend: false,
      has_pending_request: true,
    },
  ],
  total: 3,
  metadata: { timestamp: '2024-06-01T10:00:00Z', api_version: 'v1' },
};

async function setupFriendsMocks(page: Page, options: { emptyFriends?: boolean } = {}) {
  const { emptyFriends = false } = options;

  // Register social API mocks BEFORE setupDashboardMocks
  // (Playwright checks routes in reverse order, but we use specific patterns)

  // Accept friend request: POST /api/social/friends/requests/{id}/accept
  await page.route('**/api/social/friends/requests/*/accept', async (route) => {
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({ friend: mockFriends.friends[0] }),
    });
  });

  // Reject friend request: POST /api/social/friends/requests/{id}/reject
  await page.route('**/api/social/friends/requests/*/reject', async (route) => {
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({ success: true }),
    });
  });

  // Send friend request: POST /api/social/friends/requests
  await page.route('**/api/social/friends/requests', async (route, request) => {
    if (request.method() === 'POST') {
      await route.fulfill({
        status: 201,
        contentType: 'application/json',
        body: JSON.stringify({
          request: {
            id: 'req-new',
            user_id: 'user-123',
            friend_user_id: 'user-10',
            status: 'pending',
            created_at: new Date().toISOString(),
          },
        }),
      });
    } else {
      await route.fallback();
    }
  });

  // Pending friend requests: GET /api/social/friends/pending
  await page.route('**/api/social/friends/pending', async (route) => {
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify(mockPendingRequests),
    });
  });

  // Remove friend: DELETE /api/social/friends/{userId}
  // Must match /api/social/friends/{id} but NOT /api/social/friends/pending or /api/social/friends/requests
  await page.route(/\/api\/social\/friends\/user-/, async (route, request) => {
    if (request.method() === 'DELETE') {
      await route.fulfill({ status: 204 });
    } else {
      await route.fallback();
    }
  });

  // Friends list: GET /api/social/friends
  await page.route(/\/api\/social\/friends$/, async (route, request) => {
    if (request.method() === 'GET') {
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify(
          emptyFriends
            ? { friends: [], total: 0, metadata: { timestamp: '2024-06-01T10:00:00Z', api_version: 'v1' } }
            : mockFriends
        ),
      });
    } else {
      await route.fallback();
    }
  });

  // User search: GET /api/social/users/search?q=...
  await page.route('**/api/social/users/search**', async (route) => {
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify(mockSearchResults),
    });
  });

  // Social feed (needed by SocialFeedTab default view)
  await page.route('**/api/social/feed**', async (route) => {
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({ items: [], next_cursor: null, has_more: false, metadata: { timestamp: '2024-06-01T10:00:00Z', api_version: 'v1' } }),
    });
  });

  // Insight suggestions
  await page.route('**/api/social/insights/suggestions**', async (route) => {
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({ suggestions: [], total: 0, metadata: { timestamp: '2024-06-01T10:00:00Z', api_version: 'v1' } }),
    });
  });

  // Providers (needed by ChatTab)
  await page.route('**/api/providers', async (route) => {
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({ providers: [] }),
    });
  });

  // Coaches (needed by sidebar)
  await page.route('**/api/coaches**', async (route) => {
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({ coaches: [], total: 0 }),
    });
  });

  // Set up base dashboard mocks (auth, usage, etc.)
  await setupDashboardMocks(page, { role: 'user' });
}

async function navigateToFriendsTab(page: Page) {
  // Navigate to Insights tab (which contains Friends sub-view)
  await navigateToTab(page, 'Insights');

  // Click the Friends button (icon button in the SocialFeedTab toolbar)
  const friendsButton = page.getByRole('button', { name: 'Friends' });
  await friendsButton.click();
  await page.waitForTimeout(500);
}

test.describe('Friends - Friend List Display', () => {
  test.beforeEach(async ({ page }) => {
    await setupFriendsMocks(page);
    await loginToDashboard(page);
  });

  test('displays friends list with friend names', async ({ page }) => {
    await navigateToFriendsTab(page);

    await expect(page.getByText('Alice Runner')).toBeVisible({ timeout: 10000 });
    await expect(page.getByText('Bob Cyclist')).toBeVisible();
  });

  test('displays "Friends since" timestamp for each friend', async ({ page }) => {
    await navigateToFriendsTab(page);

    await expect(page.getByText('Alice Runner')).toBeVisible({ timeout: 10000 });
    await expect(page.getByText(/Friends since/i).first()).toBeVisible();
  });

  test('displays Remove button for each friend', async ({ page }) => {
    await navigateToFriendsTab(page);

    await expect(page.getByText('Alice Runner')).toBeVisible({ timeout: 10000 });
    const removeButtons = page.getByRole('button', { name: 'Remove' });
    await expect(removeButtons.first()).toBeVisible();
  });
});

test.describe('Friends - Empty State', () => {
  test('shows empty state when no friends', async ({ page }) => {
    await setupFriendsMocks(page, { emptyFriends: true });
    await loginToDashboard(page);
    await navigateToFriendsTab(page);

    await expect(page.getByText('No friends yet')).toBeVisible({ timeout: 10000 });
    await expect(page.getByText('Find and connect with other athletes')).toBeVisible();
  });

  test('empty state has Find Friends button', async ({ page }) => {
    await setupFriendsMocks(page, { emptyFriends: true });
    await loginToDashboard(page);
    await navigateToFriendsTab(page);

    await expect(page.getByText('No friends yet')).toBeVisible({ timeout: 10000 });
    await expect(page.getByRole('button', { name: 'Find Friends' }).last()).toBeVisible();
  });
});

test.describe('Friends - Remove Friend', () => {
  test.beforeEach(async ({ page }) => {
    await setupFriendsMocks(page);
    await loginToDashboard(page);
  });

  test('clicking Remove calls the remove API', async ({ page }) => {
    let removeCalled = false;
    await page.route(/\/api\/social\/friends\/user-2$/, async (route, request) => {
      if (request.method() === 'DELETE') {
        removeCalled = true;
        await route.fulfill({ status: 204 });
      } else {
        await route.fallback();
      }
    });

    await navigateToFriendsTab(page);
    await expect(page.getByText('Alice Runner')).toBeVisible({ timeout: 10000 });

    // Click the first Remove button
    await page.getByRole('button', { name: 'Remove' }).first().click();
    await page.waitForTimeout(500);

    expect(removeCalled).toBe(true);
  });
});

test.describe('Friends - Search Users', () => {
  test.beforeEach(async ({ page }) => {
    await setupFriendsMocks(page);
    await loginToDashboard(page);
  });

  test('Find Friends tab shows search input', async ({ page }) => {
    await navigateToFriendsTab(page);

    // Switch to Find Friends sub-tab (the tab button, not the empty-state button)
    const findFriendsTab = page.locator('button').filter({ hasText: 'Find Friends' }).first();
    await findFriendsTab.click();
    await page.waitForTimeout(300);

    await expect(page.getByPlaceholder('Search by name or email...')).toBeVisible({ timeout: 5000 });
    await expect(page.getByRole('button', { name: 'Search' })).toBeVisible();
  });

  test('search returns results with correct statuses', async ({ page }) => {
    await navigateToFriendsTab(page);
    const findFriendsTab = page.locator('button').filter({ hasText: 'Find Friends' }).first();
    await findFriendsTab.click();
    await page.waitForTimeout(300);

    // Enter search query and click Search
    await page.getByPlaceholder('Search by name or email...').fill('runner');
    await page.getByRole('button', { name: 'Search' }).click();

    // Verify results appear
    await expect(page.getByText('Eve Marathoner')).toBeVisible({ timeout: 10000 });
    await expect(page.getByText('Alice Runner')).toBeVisible();
    await expect(page.getByText('Frank Sprinter')).toBeVisible();
  });

  test('Add Friend button appears for non-friend users', async ({ page }) => {
    await navigateToFriendsTab(page);
    const findFriendsTab = page.locator('button').filter({ hasText: 'Find Friends' }).first();
    await findFriendsTab.click();
    await page.waitForTimeout(300);

    await page.getByPlaceholder('Search by name or email...').fill('eve');
    await page.getByRole('button', { name: 'Search' }).click();

    await expect(page.getByText('Eve Marathoner')).toBeVisible({ timeout: 10000 });
    await expect(page.getByRole('button', { name: 'Add Friend' })).toBeVisible();
  });

  test('clicking Add Friend sends friend request', async ({ page }) => {
    let requestSent = false;
    await page.route('**/api/social/friends/requests', async (route, request) => {
      if (request.method() === 'POST') {
        requestSent = true;
        await route.fulfill({
          status: 201,
          contentType: 'application/json',
          body: JSON.stringify({
            request: {
              id: 'req-new',
              user_id: 'user-123',
              friend_user_id: 'user-10',
              status: 'pending',
              created_at: new Date().toISOString(),
            },
          }),
        });
      } else {
        await route.fallback();
      }
    });

    await navigateToFriendsTab(page);
    const findFriendsTab = page.locator('button').filter({ hasText: 'Find Friends' }).first();
    await findFriendsTab.click();
    await page.waitForTimeout(300);

    await page.getByPlaceholder('Search by name or email...').fill('eve');
    await page.getByRole('button', { name: 'Search' }).click();

    await expect(page.getByText('Eve Marathoner')).toBeVisible({ timeout: 10000 });
    await page.getByRole('button', { name: 'Add Friend' }).click();
    await page.waitForTimeout(500);

    expect(requestSent).toBe(true);
  });
});

test.describe('Friends - Pending Requests', () => {
  test.beforeEach(async ({ page }) => {
    await setupFriendsMocks(page);
    await loginToDashboard(page);
  });

  test('Pending tab shows received and sent requests', async ({ page }) => {
    await navigateToFriendsTab(page);
    const pendingTab = page.locator('button').filter({ hasText: 'Pending' });
    await pendingTab.click();
    await page.waitForTimeout(500);

    // Should show received request user name
    await expect(page.getByText('Carol Swimmer')).toBeVisible({ timeout: 10000 });
    // Should show sent request user name
    await expect(page.getByText('Dave Triathlete')).toBeVisible();
  });

  test('received request shows Accept and Decline buttons', async ({ page }) => {
    await navigateToFriendsTab(page);
    const pendingTab = page.locator('button').filter({ hasText: 'Pending' });
    await pendingTab.click();
    await page.waitForTimeout(500);

    await expect(page.getByText('Carol Swimmer')).toBeVisible({ timeout: 10000 });
    await expect(page.getByRole('button', { name: 'Accept' })).toBeVisible();
    await expect(page.getByRole('button', { name: 'Decline' })).toBeVisible();
  });

  test('sent request shows Awaiting status', async ({ page }) => {
    await navigateToFriendsTab(page);
    const pendingTab = page.locator('button').filter({ hasText: 'Pending' });
    await pendingTab.click();
    await page.waitForTimeout(500);

    await expect(page.getByText('Dave Triathlete')).toBeVisible({ timeout: 10000 });
    await expect(page.getByText('Awaiting')).toBeVisible();
  });

  test('clicking Accept calls the accept API', async ({ page }) => {
    let acceptCalled = false;
    await page.route('**/api/social/friends/requests/req-1/accept', async (route) => {
      acceptCalled = true;
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({ friend: mockFriends.friends[0] }),
      });
    });

    await navigateToFriendsTab(page);
    const pendingTab = page.locator('button').filter({ hasText: 'Pending' });
    await pendingTab.click();
    await page.waitForTimeout(500);

    await expect(page.getByText('Carol Swimmer')).toBeVisible({ timeout: 10000 });
    await page.getByRole('button', { name: 'Accept' }).click();
    await page.waitForTimeout(500);

    expect(acceptCalled).toBe(true);
  });

  test('clicking Decline calls the reject API', async ({ page }) => {
    let declineCalled = false;
    await page.route('**/api/social/friends/requests/req-1/reject', async (route) => {
      declineCalled = true;
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({ success: true }),
      });
    });

    await navigateToFriendsTab(page);
    const pendingTab = page.locator('button').filter({ hasText: 'Pending' });
    await pendingTab.click();
    await page.waitForTimeout(500);

    await expect(page.getByText('Carol Swimmer')).toBeVisible({ timeout: 10000 });
    await page.getByRole('button', { name: 'Decline' }).click();
    await page.waitForTimeout(500);

    expect(declineCalled).toBe(true);
  });
});

test.describe('Friends - Tab Navigation', () => {
  test.beforeEach(async ({ page }) => {
    await setupFriendsMocks(page);
    await loginToDashboard(page);
  });

  test('can switch between Friends, Find Friends, and Pending tabs', async ({ page }) => {
    await navigateToFriendsTab(page);

    // Friends tab (default) - shows friend list
    await expect(page.getByText('Alice Runner')).toBeVisible({ timeout: 10000 });

    // Switch to Find Friends
    const findFriendsTab = page.locator('button').filter({ hasText: 'Find Friends' }).first();
    await findFriendsTab.click();
    await page.waitForTimeout(300);
    await expect(page.getByPlaceholder('Search by name or email...')).toBeVisible();

    // Switch to Pending
    const pendingTab = page.locator('button').filter({ hasText: 'Pending' });
    await pendingTab.click();
    await page.waitForTimeout(500);
    await expect(page.getByText('Carol Swimmer')).toBeVisible({ timeout: 10000 });

    // Switch back to Friends
    const friendsTab = page.locator('button').filter({ hasText: /^Friends$/ }).first();
    if (await friendsTab.isVisible().catch(() => false)) {
      await friendsTab.click();
      await page.waitForTimeout(500);
      await expect(page.getByText('Alice Runner')).toBeVisible({ timeout: 10000 });
    }
  });
});

test.describe('Friends - Error Handling', () => {
  test('handles network failure on friend list load', async ({ page }) => {
    await setupFriendsMocks(page);

    // Override friends endpoint with error
    await page.route(/\/api\/social\/friends$/, async (route) => {
      await route.fulfill({
        status: 500,
        contentType: 'application/json',
        body: JSON.stringify({ error: 'Internal server error' }),
      });
    });

    await loginToDashboard(page);
    await navigateToFriendsTab(page);

    // Page should still render without crashing
    await page.waitForTimeout(1000);
    await expect(page.locator('main')).toBeVisible();
  });

  test('handles network failure on search', async ({ page }) => {
    await setupFriendsMocks(page);

    // Override search endpoint with error
    await page.route('**/api/social/users/search**', async (route) => {
      await route.fulfill({
        status: 500,
        contentType: 'application/json',
        body: JSON.stringify({ error: 'Search failed' }),
      });
    });

    await loginToDashboard(page);
    await navigateToFriendsTab(page);
    const findFriendsTab = page.locator('button').filter({ hasText: 'Find Friends' }).first();
    await findFriendsTab.click();
    await page.waitForTimeout(300);

    await page.getByPlaceholder('Search by name or email...').fill('test');
    await page.getByRole('button', { name: 'Search' }).click();
    await page.waitForTimeout(500);

    // Should not crash - page should remain functional
    await expect(page.getByPlaceholder('Search by name or email...')).toBeVisible();
  });
});
