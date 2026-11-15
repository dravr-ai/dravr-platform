// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Playwright E2E tests for the admin impersonation feature.
// ABOUTME: Tests impersonation start/end, banner display, and role-based visibility.

import { test, expect, type Page } from '@playwright/test';
import { applyTestStubs, loginToDashboard, setupDashboardMocks, signInThroughHostedPage } from './test-helpers';

// Helper to authenticate as an admin and navigate to users tab
async function loginAsAdminAndNavigateToUsers(page: Page) {
  await applyTestStubs(page);
  // Mock setup status
  await page.route('**/admin/setup/status', async (route) => {
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({ needs_setup: false, admin_user_exists: true }),
    });
  });

  // Mock the hosted sign-in's code exchange - ADMIN role
  await page.route('**/oauth/token', async (route) => {
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({
        access_token: 'test-jwt-token',
        token_type: 'Bearer',
        expires_in: 86400,
        csrf_token: 'test-csrf-token',
        user: {
          user_id: 'admin-1',
          email: 'admin@test.com',
          display_name: 'Admin One',
          role: 'admin',
          is_admin: true,
          user_status: 'active',
          tier: 'enterprise',
        },
      }),
    });
  });

  // Mock analytics
  await page.route('**/api/dashboard/analytics**', async (route) => {
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({ daily_usage: [] }),
    });
  });

  await page.goto('/');
  await signInThroughHostedPage(page);

  // Wait for dashboard sidebar to appear - the sidebar contains Dravr logo
  await page.waitForSelector('text=Dravr', { timeout: 10000 });
  await page.waitForTimeout(300);
}

test.describe('Impersonation - Admin Access', () => {
  test.beforeEach(async ({ page }) => {
    // Mock pending users
    await page.route('**/api/admin/pending-users', async (route) => {
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({ success: true, message: 'Retrieved 0 pending users', data: { count: 0, users: [] } }),
      });
    });

    // The users list is read once, as the admin lands on Users, so every row a
    // test opens is in it from the start: a user who can be impersonated,
    // another admin, and an account still pending.
    // Each row carries `role` and the `is_admin` derived from it, as the
    // server's list does (`UserSummary`).
    await page.route('**/api/admin/users**', async (route) => {
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({
          success: true,
          message: 'Retrieved users',
          data: {
            users: [
              {
                id: 'user-1',
                email: 'testuser@example.com',
                display_name: 'Test User',
                role: 'user',
                is_admin: false,
                user_status: 'active',
                tier: 'starter',
                created_at: '2024-01-15T10:00:00Z',
                last_active: '2024-01-20T15:30:00Z',
              },
              {
                id: 'admin-2',
                email: 'otheradmin@example.com',
                display_name: 'Other Admin',
                role: 'admin',
                is_admin: true,
                user_status: 'active',
                tier: 'professional',
                created_at: '2024-01-10T10:00:00Z',
                last_active: '2024-01-20T15:30:00Z',
              },
              {
                id: 'pending-user-1',
                email: 'pending@example.com',
                display_name: 'Pending User',
                role: 'user',
                is_admin: false,
                user_status: 'pending',
                tier: 'trial',
                created_at: '2024-01-15T10:00:00Z',
                last_active: null,
              },
            ],
            total: 3,
            has_more: false,
          },
        }),
      });
    });

    // Mock rate limit endpoint for user details
    await page.route('**/admin/users/*/rate-limit', async (route) => {
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({
          user_id: 'user-1',
          tier: 'starter',
          rate_limits: {
            monthly: { limit: 10000, used: 500, remaining: 9500 },
          },
          reset_times: {
            monthly_reset: '2024-02-01T00:00:00Z',
          },
        }),
      });
    });

    // Mock activity endpoint
    await page.route('**/admin/users/*/activity**', async (route) => {
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({
          user_id: 'user-1',
          period_days: 30,
          total_requests: 500,
          top_tools: [],
        }),
      });
    });

    // Admin profile endpoint backs the Installed Coaches + Groups + Coach Style
    // sections of UserDetailDrawer. Without a mock the un-authed request 401s and
    // leaves the new cards in skeleton state, pushing the Impersonate button
    // out of viewport so toBeVisible() times out.
    await page.route('**/api/admin/users/*/admin-profile**', async (route) => {
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({
          user_id: 'user-1',
          coaching_persona: 'casual',
          default_coach_id: null,
          installed_agents: [],
          joined_groups: [],
        }),
      });
    });

    await loginAsAdminAndNavigateToUsers(page);
  });

  test('admin sees the Admin role in sidebar', async ({ page }) => {
    // The role indicator under the name, not the name itself
    await expect(page.locator('aside').getByText('Admin', { exact: true }).first()).toBeVisible();
  });

  test('admin sees Users tab', async ({ page }) => {
    // Navigate to Users tab - use button role for sidebar navigation
    const usersTab = page.locator('button', { hasText: 'Users' }).first();
    await usersTab.click();
    await expect(page.locator('h1').filter({ hasText: 'Users' })).toBeVisible({ timeout: 5000 });
  });

  test('admin sees Impersonate User button for active regular users', async ({ page }) => {
    await page.locator('button', { hasText: 'Users' }).first().click();
    await page.waitForTimeout(500);

    // Click on Active tab to see active users
    // Click the Active tab button in the user management tabs
    await page.locator('button', { hasText: 'Active' }).first().click();
    await page.waitForTimeout(300);

    // Click on a user to open details drawer - look for the user row
    const userRow = page.getByText('testuser@example.com').first();
    await userRow.click();
    await page.waitForTimeout(500);

    // Should see Impersonate User button in the drawer
    await expect(page.getByRole('button', { name: /Impersonate User/i })).toBeVisible();
  });

  test('impersonate button not shown for admin users', async ({ page }) => {
    await page.locator('button', { hasText: 'Users' }).first().click();
    await page.locator('button', { hasText: 'Active' }).first().click();

    // Open the other admin
    await page.getByText('otheradmin@example.com').first().click();

    // The drawer is open on that account, down to its actions
    const drawer = page.getByRole('dialog', { name: 'User Details' });
    await expect(drawer.getByText('otheradmin@example.com')).toBeVisible();
    await expect(drawer.getByRole('button', { name: 'Reset Password' })).toBeVisible();

    // An admin is never impersonated
    await expect(page.getByRole('button', { name: /Impersonate User/i })).toHaveCount(0);
  });

  test('impersonate button not shown for pending users', async ({ page }) => {
    await page.locator('button', { hasText: 'Users' }).first().click();
    await page.locator('button', { hasText: 'All Users' }).first().click();

    // Open the pending account
    await page.getByText('pending@example.com').first().click();

    // The drawer is open on that account, down to its actions
    const drawer = page.getByRole('dialog', { name: 'User Details' });
    await expect(drawer.getByText('pending@example.com')).toBeVisible();
    await expect(drawer.getByRole('button', { name: 'Reset Password' })).toBeVisible();

    // Only an active account is impersonated
    await expect(page.getByRole('button', { name: /Impersonate User/i })).toHaveCount(0);
  });
});

test.describe('Impersonation - Start and End Flow', () => {
  test.beforeEach(async ({ page }) => {
    await page.route('**/api/admin/pending-users', async (route) => {
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({ success: true, message: 'Retrieved 0 pending users', data: { count: 0, users: [] } }),
      });
    });

    await page.route('**/api/admin/users**', async (route) => {
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({
          success: true,
          message: 'Retrieved users',
          data: {
            users: [
              {
                id: 'user-1',
                email: 'testuser@example.com',
                display_name: 'Test User',
                role: 'user',
                user_status: 'active',
                tier: 'starter',
                created_at: '2024-01-15T10:00:00Z',
                last_active: '2024-01-20T15:30:00Z',
              },
            ],
            total: 1,
            has_more: false,
          },
        }),
      });
    });

    await page.route('**/admin/users/*/rate-limit', async (route) => {
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({
          user_id: 'user-1',
          tier: 'starter',
          rate_limits: {
            monthly: { limit: 10000, used: 500, remaining: 9500 },
          },
          reset_times: {
            monthly_reset: '2024-02-01T00:00:00Z',
          },
        }),
      });
    });

    await page.route('**/admin/users/*/activity**', async (route) => {
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({
          user_id: 'user-1',
          period_days: 30,
          total_requests: 500,
          top_tools: [],
        }),
      });
    });

    // Admin profile endpoint backs the new Installed Coaches + Groups + Coach
    // Style cards in UserDetailDrawer. Without a mock the un-authed request
    // 401s and leaves the cards in skeleton state, pushing the Impersonate
    // button below the viewport so Playwright's toBeVisible() times out.
    await page.route('**/api/admin/users/*/admin-profile**', async (route) => {
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({
          user_id: 'user-1',
          coaching_persona: 'casual',
          default_coach_id: null,
          installed_agents: [],
          joined_groups: [],
        }),
      });
    });

    await loginAsAdminAndNavigateToUsers(page);
  });

  test('can start impersonation and see banner', async ({ page }) => {
    // Mock impersonate endpoint
    await page.route('**/api/admin/impersonate', async (route) => {
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({
          session_id: 'session-123',
          target_user: {
            id: 'user-1',
            email: 'testuser@example.com',
            display_name: 'Test User',
            role: 'user',
          },
          impersonation_token: 'impersonation-jwt-token',
        }),
      });
    });

    await page.locator('button', { hasText: 'Users' }).first().click();
    await page.waitForTimeout(500);

    // Click on Active tab
    // Click the Active tab button in the user management tabs
    await page.locator('button', { hasText: 'Active' }).first().click();
    await page.waitForTimeout(300);

    // Click on user to open details
    const userRow = page.getByText('testuser@example.com').first();
    await userRow.click();
    await page.waitForTimeout(500);

    // Click Impersonate User button
    const impersonateButton = page.getByRole('button', { name: /Impersonate User/i });
    await impersonateButton.click();
    await page.waitForTimeout(500);

    // Should see impersonation banner
    await expect(page.getByText(/You are impersonating/i)).toBeVisible();
    await expect(page.getByText('Test User').first()).toBeVisible();
    await expect(page.getByRole('button', { name: /End Impersonation/i })).toBeVisible();
  });

  test('can end impersonation', async ({ page }) => {
    // Mock impersonate endpoint
    await page.route('**/api/admin/impersonate', async (route) => {
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({
          session_id: 'session-123',
          target_user: {
            id: 'user-1',
            email: 'testuser@example.com',
            display_name: 'Test User',
            role: 'user',
          },
          impersonation_token: 'impersonation-jwt-token',
        }),
      });
    });

    // Mock end impersonation endpoint
    await page.route('**/api/admin/impersonate/end', async (route) => {
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({
          success: true,
          message: 'Impersonation session ended',
        }),
      });
    });

    await page.locator('button', { hasText: 'Users' }).first().click();
    await page.waitForTimeout(500);

    // Click the Active tab button in the user management tabs
    await page.locator('button', { hasText: 'Active' }).first().click();
    await page.waitForTimeout(300);

    const userRow = page.getByText('testuser@example.com').first();
    await userRow.click();
    await page.waitForTimeout(500);

    // Start impersonation
    const impersonateButton = page.getByRole('button', { name: /Impersonate User/i });
    await impersonateButton.click();
    await page.waitForTimeout(500);

    // Verify banner is visible
    await expect(page.getByText(/You are impersonating/i)).toBeVisible();

    // End impersonation
    const endButton = page.getByRole('button', { name: /End Impersonation/i });
    await endButton.click();
    await page.waitForTimeout(500);

    // Banner should be gone
    await expect(page.getByText(/You are impersonating/i)).not.toBeVisible();
  });

  test('impersonation banner shows target user info', async ({ page }) => {
    await page.route('**/api/admin/impersonate', async (route) => {
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({
          session_id: 'session-123',
          target_user: {
            id: 'user-1',
            email: 'testuser@example.com',
            display_name: 'Test User',
            role: 'user',
          },
          impersonation_token: 'impersonation-jwt-token',
        }),
      });
    });

    await page.locator('button', { hasText: 'Users' }).first().click();
    await page.waitForTimeout(500);

    // Click the Active tab button in the user management tabs
    await page.locator('button', { hasText: 'Active' }).first().click();
    await page.waitForTimeout(300);

    const userRow = page.getByText('testuser@example.com').first();
    await userRow.click();
    await page.waitForTimeout(500);

    const impersonateButton = page.getByRole('button', { name: /Impersonate User/i });
    await impersonateButton.click();
    await page.waitForTimeout(500);

    // Banner should show user info
    await expect(page.getByText('Test User').first()).toBeVisible();
    await expect(page.getByText('testuser@example.com').first()).toBeVisible();
    await expect(page.getByText(/Role: user/i)).toBeVisible();
  });
});

test.describe('Impersonation - A User Cannot Impersonate', () => {
  test('a user without the admin role has no Users screen and no Impersonate User button', async ({ page }) => {
    await setupDashboardMocks(page, { role: 'user' });
    await loginToDashboard(page);

    await expect(page.locator('aside').getByRole('button', { name: 'Users', exact: true })).toHaveCount(0);

    // A typed admin hash resolves to Home instead of mounting the Users screen.
    await page.evaluate(() => {
      window.location.hash = '#users';
    });
    await expect(page).toHaveURL(/#home$/);
    await expect(page.getByRole('button', { name: /Impersonate User/i })).toHaveCount(0);
  });
});

test.describe('Impersonation - Error Handling', () => {
  test.beforeEach(async ({ page }) => {
    await page.route('**/api/admin/pending-users', async (route) => {
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({ success: true, message: 'Retrieved 0 pending users', data: { count: 0, users: [] } }),
      });
    });

    await page.route('**/api/admin/users**', async (route) => {
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({
          success: true,
          message: 'Retrieved users',
          data: {
            users: [
              {
                id: 'user-1',
                email: 'testuser@example.com',
                display_name: 'Test User',
                role: 'user',
                user_status: 'active',
                tier: 'starter',
                created_at: '2024-01-15T10:00:00Z',
                last_active: '2024-01-20T15:30:00Z',
              },
            ],
            total: 1,
            has_more: false,
          },
        }),
      });
    });

    await page.route('**/admin/users/*/rate-limit', async (route) => {
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({
          user_id: 'user-1',
          tier: 'starter',
          rate_limits: {
            monthly: { limit: 10000, used: 500, remaining: 9500 },
          },
          reset_times: {
            monthly_reset: '2024-02-01T00:00:00Z',
          },
        }),
      });
    });

    await page.route('**/admin/users/*/activity**', async (route) => {
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({
          user_id: 'user-1',
          period_days: 30,
          total_requests: 500,
          top_tools: [],
        }),
      });
    });

    // Admin profile endpoint backs the new Installed Coaches + Groups + Coach
    // Style cards in UserDetailDrawer. Without a mock the un-authed request
    // 401s and leaves the cards in skeleton state, pushing the Impersonate
    // button below the viewport so Playwright's toBeVisible() times out.
    await page.route('**/api/admin/users/*/admin-profile**', async (route) => {
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({
          user_id: 'user-1',
          coaching_persona: 'casual',
          default_coach_id: null,
          installed_agents: [],
          joined_groups: [],
        }),
      });
    });

    await loginAsAdminAndNavigateToUsers(page);
  });

  test('handles impersonation API error gracefully', async ({ page }) => {
    // Mock impersonate endpoint with error
    await page.route('**/api/admin/impersonate', async (route) => {
      await route.fulfill({
        status: 403,
        contentType: 'application/json',
        body: JSON.stringify({
          error: 'Not authorized to impersonate this user',
        }),
      });
    });

    await page.locator('button', { hasText: 'Users' }).first().click();
    await page.waitForTimeout(500);

    // Click the Active tab button in the user management tabs
    await page.locator('button', { hasText: 'Active' }).first().click();
    await page.waitForTimeout(300);

    const userRow = page.getByText('testuser@example.com').first();
    await userRow.click();
    await page.waitForTimeout(500);

    const impersonateButton = page.getByRole('button', { name: /Impersonate User/i });
    await impersonateButton.click();
    await page.waitForTimeout(500);

    // Should show error and NOT show impersonation banner
    await expect(page.getByText(/You are impersonating/i)).not.toBeVisible();
  });
});
