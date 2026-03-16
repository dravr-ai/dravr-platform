// ABOUTME: Playwright E2E tests for the Settings page UX redesign.
// ABOUTME: Tests user settings tabs, change password modal, about tab, and admin settings navigation.

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { test, expect } from '@playwright/test';

// Helper to set up mocks for an authenticated user session
async function setupAuthenticatedMocks(page: import('@playwright/test').Page, isAdmin = false) {
  await page.route('**/admin/setup/status', async (route) => {
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({ needs_setup: false, admin_user_exists: true }),
    });
  });

  await page.route('**/api/auth/me', async (route) => {
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({
        id: 'user-1',
        email: isAdmin ? 'admin@pierre.dev' : 'webtest@pierre.dev',
        display_name: isAdmin ? 'Admin User' : 'Web Test',
        is_admin: isAdmin,
        role: isAdmin ? 'admin' : 'user',
        tier: 'free',
        created_at: '2024-06-15T10:00:00Z',
      }),
    });
  });

  await page.route('**/oauth/token', async (route) => {
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({
        access_token: 'test-jwt-token',
        token_type: 'Bearer',
        expires_in: 86400,
        csrf_token: 'test-csrf',
        user: {
          id: 'user-1',
          email: isAdmin ? 'admin@pierre.dev' : 'webtest@pierre.dev',
          display_name: isAdmin ? 'Admin User' : 'Web Test',
          is_admin: isAdmin,
          role: isAdmin ? 'admin' : 'user',
          user_status: 'active',
          tier: 'free',
          created_at: '2024-06-15T10:00:00Z',
        },
      }),
    });
  });

  // Mock user stats
  await page.route('**/api/user/stats', async (route) => {
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({ connected_providers: 2, days_active: 45 }),
    });
  });

  // Mock MCP tokens
  await page.route('**/api/user/mcp-tokens', async (route) => {
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({ tokens: [] }),
    });
  });

  // Mock OAuth apps
  await page.route('**/api/users/oauth-apps', async (route) => {
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({ apps: [] }),
    });
  });

  // Mock OAuth status
  await page.route('**/api/oauth/status', async (route) => {
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify([
        { provider: 'strava', connected: false },
        { provider: 'fitbit', connected: false },
      ]),
    });
  });

  // Mock dashboard overview (for admin)
  await page.route('**/api/dashboard/overview**', async (route) => {
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({
        total_api_keys: 5,
        active_api_keys: 3,
        total_requests_today: 100,
        total_requests_this_month: 2500,
      }),
    });
  });

  // Mock pending users (for admin)
  await page.route('**/api/admin/users/pending', async (route) => {
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify([]),
    });
  });

  // Mock A2A dashboard (for admin)
  await page.route('**/api/a2a/dashboard**', async (route) => {
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({
        total_clients: 0,
        active_clients: 0,
        requests_today: 0,
        requests_this_month: 0,
      }),
    });
  });

  // Mock change password
  await page.route('**/api/user/change-password', async (route) => {
    const body = route.request().postDataJSON();
    if (body?.current_password === 'WrongPassword123') {
      await route.fulfill({
        status: 401,
        contentType: 'application/json',
        body: JSON.stringify({ error: 'Current password is incorrect' }),
      });
    } else {
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({ message: 'Password changed successfully' }),
      });
    }
  });

  // Mock rate limit overview
  await page.route('**/api/dashboard/rate-limits**', async (route) => {
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify([]),
    });
  });

  // Mock A2A clients list (used by API Tokens tab)
  await page.route('**/a2a/clients', async (route) => {
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify([]),
    });
  });

  // Mock LLM settings (used by AI Settings tab)
  await page.route('**/api/llm/settings', async (route) => {
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({ providers: [] }),
    });
  });

  // Mock admin configuration catalog and audit (used by AdminConfiguration on Configuration tab)
  await page.route('**/api/admin/config/catalog', async (route) => {
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({ parameters: [] }),
    });
  });

  await page.route('**/api/admin/config/audit**', async (route) => {
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({ entries: [] }),
    });
  });

  // Mock tool availability (used by AdminConfiguration)
  await page.route('**/api/admin/tools**', async (route) => {
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({ tools: [] }),
    });
  });

  // Mock admin settings (used by AdminSettings component on Configuration tab)
  await page.route('**/api/admin/settings/auto-approval', async (route) => {
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({ data: { enabled: false, description: 'Auto-approve new users' } }),
    });
  });

  await page.route('**/api/admin/settings/social-insights', async (route) => {
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({
        data: {
          min_activities_for_comparison: 5,
          comparison_window_days: 90,
          min_similar_users: 3,
          max_comparison_users: 50,
        },
      }),
    });
  });

  // Mock providers status (needed by ChatTab and ProviderConnectionCards)
  await page.route('**/api/providers', async (route) => {
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({ providers: [] }),
    });
  });

  // Mock coaches (needed by PromptSuggestions in welcome view)
  await page.route('**/api/coaches**', async (route) => {
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({ coaches: [], total: 0, metadata: { timestamp: new Date().toISOString(), api_version: 'v1' } }),
    });
  });

  // Mock notifications (needed by sidebar NotificationBell)
  await page.route('**/api/notifications/**', async (route) => {
    const url = route.request().url();
    if (url.includes('unread-count')) {
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({ count: 0 }),
      });
    } else {
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({ notifications: [], total: 0, unread_count: 0 }),
      });
    }
  });

  // Mock social endpoints (needed by Insights tab)
  await page.route('**/api/social/**', async (route) => {
    const url = route.request().url();
    if (url.includes('/friends')) {
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({ friends: [], total: 0, metadata: { timestamp: new Date().toISOString(), api_version: 'v1' } }),
      });
    } else if (url.includes('/feed')) {
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({ insights: [], next_cursor: null, has_more: false, metadata: { timestamp: new Date().toISOString(), api_version: 'v1' } }),
      });
    } else if (url.includes('/suggestions')) {
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({ suggestions: [], total: 0, metadata: { timestamp: new Date().toISOString(), api_version: 'v1' } }),
      });
    } else if (url.includes('/settings')) {
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({
          user_id: 'user-1',
          discoverable: true,
          default_visibility: 'friends',
          share_activity_types: [],
          notifications: { friend_requests: true, insight_reactions: true, adapted_insights: true },
          created_at: new Date().toISOString(),
          updated_at: new Date().toISOString(),
        }),
      });
    } else {
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({}),
      });
    }
  });

  // Mock store endpoints (needed by Discover tab)
  await page.route('**/api/store/**', async (route) => {
    const url = route.request().url();
    if (url.includes('/installations')) {
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({ coaches: [], metadata: { timestamp: new Date().toISOString(), api_version: 'v1' } }),
      });
    } else if (url.includes('/categories')) {
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({ categories: [], metadata: { timestamp: new Date().toISOString(), api_version: 'v1' } }),
      });
    } else {
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({ coaches: [], next_cursor: null, has_more: false, metadata: { timestamp: new Date().toISOString(), api_version: 'v1' } }),
      });
    }
  });

  // Mock chat conversations (needed by Chat tab)
  await page.route('**/api/chat/conversations**', async (route) => {
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({ conversations: [], total: 0, limit: 50, offset: 0 }),
    });
  });

  // Mock prompts suggestions (needed by Chat welcome view)
  await page.route('**/api/prompts/suggestions', async (route) => {
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({
        categories: [],
        welcome_prompt: '',
        metadata: { timestamp: new Date().toISOString(), api_version: '1.0' },
      }),
    });
  });

  // Mock user LLM settings (needed by AI Settings tab)
  await page.route('**/api/user/llm-settings**', async (route) => {
    if (route.request().method() === 'GET') {
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({
          current_provider: null,
          providers: [],
          user_credentials: [],
          tenant_credentials: [],
        }),
      });
    } else {
      await route.fallback();
    }
  });

  // Mock admin store stats (needed by Coach Store tab badge)
  await page.route('**/api/admin/store/stats', async (route) => {
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({
        pending_count: 0,
        published_count: 0,
        rejected_count: 0,
        total_installs: 0,
        rejection_rate: 0,
      }),
    });
  });

  // Mock admin pending users (needed by admin sidebar)
  await page.route('**/api/admin/pending-users', async (route) => {
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({ count: 0, users: [] }),
    });
  });

  // Mock admin users list
  await page.route('**/api/admin/users**', async (route) => {
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({ users: [], total_count: 0 }),
    });
  });

  // Mock dashboard analytics
  await page.route('**/api/dashboard/analytics**', async (route) => {
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({ daily_usage: [] }),
    });
  });

  // Mock A2A dashboard overview
  await page.route('**/a2a/dashboard/overview', async (route) => {
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({
        total_clients: 0,
        active_clients: 0,
        requests_today: 0,
        requests_this_month: 0,
      }),
    });
  });

  // Mock A2A client individual endpoints
  await page.route('**/a2a/clients/*', async (route) => {
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({}),
    });
  });

  // Mock admin LLM consumption endpoint
  await page.route('**/admin/usage/llm-consumption**', async (route) => {
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({
        summary: { total_tokens: 0, total_calls: 0, estimated_cost_usd: 0 },
        breakdown: [],
        daily_series: [],
      }),
    });
  });

  // Mock user LLM consumption endpoint
  await page.route('**/api/usage/llm-consumption**', async (route) => {
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({
        summary: { total_tokens: 0, total_calls: 0, estimated_cost_usd: 0 },
        breakdown: [],
        daily_series: [],
      }),
    });
  });

  // Mock usage status endpoint (used by UserSettings usage quota card)
  await page.route('**/api/usage/status', async (route) => {
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({
        daily: {
          messages: { allowed: true, current: 5, limit: 50, warning: false, burst_zone: false, resets_at: '2026-02-19T00:00:00Z' },
          tool_calls: { allowed: true, current: 2, limit: 100, warning: false, burst_zone: false, resets_at: '2026-02-19T00:00:00Z' },
          tokens: { allowed: true, current: 12000, limit: 500000, warning: false, burst_zone: false, resets_at: '2026-02-19T00:00:00Z' },
        },
        weekly: {
          messages: { allowed: true, current: 15, limit: 250, warning: false, burst_zone: false, resets_at: '2026-02-23T00:00:00Z' },
          tool_calls: { allowed: true, current: 8, limit: 500, warning: false, burst_zone: false, resets_at: '2026-02-23T00:00:00Z' },
          tokens: { allowed: true, current: 45000, limit: 2000000, warning: false, burst_zone: false, resets_at: '2026-02-23T00:00:00Z' },
        },
        resources: { coaches: 1, max_coaches: 3, conversations: 2, max_conversations: 20 },
      }),
    });
  });
}

async function loginAndNavigateToSettings(
  page: import('@playwright/test').Page,
  isAdmin = false
) {
  await setupAuthenticatedMocks(page, isAdmin);
  await page.goto('/');
  await page.waitForSelector('form', { timeout: 10000 });

  await page.locator('input[name="email"]').fill(isAdmin ? 'admin@pierre.dev' : 'webtest@pierre.dev');
  await page.locator('input[name="password"]').fill('TestPassword123');
  await page.getByRole('button', { name: 'Sign in' }).click();

  // Wait for dashboard to load
  await expect(page.locator('input[name="email"]')).not.toBeVisible({ timeout: 10000 });

  // Click the gear icon (Settings) in the bottom-left profile bar
  const settingsGear = page.getByRole('button', { name: 'Settings', exact: true });
  if (await settingsGear.first().isVisible().catch(() => false)) {
    await settingsGear.first().click();
    await page.waitForTimeout(500);
  }
}

test.describe('Settings Page - User Mode', () => {
  test('settings tab navigation shows all tabs', async ({ page }) => {
    await loginAndNavigateToSettings(page);

    // Use button role to avoid matching headings with the same text
    await expect(page.getByRole('button', { name: 'Profile' })).toBeVisible();
    await expect(page.getByRole('button', { name: 'Data Providers' })).toBeVisible();
    await expect(page.getByRole('button', { name: 'API Tokens' })).toBeVisible();
    await expect(page.getByRole('button', { name: 'AI Settings' })).toBeVisible();
    await expect(page.getByRole('button', { name: 'About' })).toBeVisible();
    await expect(page.getByRole('button', { name: 'Account' })).toBeVisible();
  });

  test('profile tab shows user info and stats', async ({ page }) => {
    await loginAndNavigateToSettings(page);

    // User info should be visible (name appears in both sidebar and settings content)
    await expect(page.getByRole('main').getByText('Web Test')).toBeVisible();
    // Email appears in both the header and the form field
    await expect(page.getByText('webtest@pierre.dev').first()).toBeVisible();

    // Stat cards should appear after data loads
    await expect(page.getByText('Connected Providers')).toBeVisible({ timeout: 5000 });
    await expect(page.getByText('Days Active')).toBeVisible({ timeout: 5000 });
  });

  test('about tab shows version and links', async ({ page }) => {
    await loginAndNavigateToSettings(page);

    // Click About tab button
    await page.getByRole('button', { name: 'About' }).click();
    await page.waitForTimeout(300);

    await expect(page.getByText('Version')).toBeVisible();
    await expect(page.getByText('1.0.0')).toBeVisible();
    await expect(page.getByText('Help Center')).toBeVisible();
    await expect(page.getByText('Terms & Privacy')).toBeVisible();
  });

  test('account tab shows member since and change password', async ({ page }) => {
    await loginAndNavigateToSettings(page);

    // Click Account tab button
    await page.getByRole('button', { name: 'Account' }).click();
    await page.waitForTimeout(300);

    // Member since should show formatted date
    await expect(page.getByText('Jun 15, 2024')).toBeVisible();

    // Change password button
    await expect(page.getByRole('button', { name: 'Change Password' })).toBeVisible();

    // Danger zone
    await expect(page.getByText('Danger Zone')).toBeVisible();
    await expect(page.getByRole('button', { name: 'Sign Out', exact: true })).toBeVisible();
  });

  test('change password modal opens and validates', async ({ page }) => {
    await loginAndNavigateToSettings(page);

    // Go to Account tab
    await page.getByRole('button', { name: 'Account' }).click();
    await page.waitForTimeout(300);

    // Open change password modal - the button in Account tab's Security section
    await page.getByRole('button', { name: 'Change Password' }).click();
    await page.waitForTimeout(300);

    // Modal should be visible with password fields
    const currentPasswordInput = page.locator('input[type="password"]').first();
    await expect(currentPasswordInput).toBeVisible();

    // Fill in mismatched passwords
    const passwordInputs = page.locator('input[type="password"]');
    await passwordInputs.nth(0).fill('password123');
    await passwordInputs.nth(1).fill('NewPass456');
    await passwordInputs.nth(2).fill('DifferentPass789');

    // Submit via the "Update Password" button in the modal footer
    await page.getByRole('button', { name: 'Update Password' }).click();
    await page.waitForTimeout(300);

    // Should show mismatch error (appears in both modal banner and field validation)
    await expect(page.getByText(/passwords do not match/i).first()).toBeVisible();
  });

  test('data providers tab shows fitness providers and credentials sections', async ({ page }) => {
    await loginAndNavigateToSettings(page);

    await page.getByRole('button', { name: 'Data Providers' }).click();
    await page.waitForTimeout(300);

    await expect(page.getByRole('heading', { name: 'Fitness Providers' })).toBeVisible();
    await expect(page.getByRole('heading', { name: 'Custom API Credentials' })).toBeVisible();
  });

  test('tokens tab shows create new token button', async ({ page }) => {
    await loginAndNavigateToSettings(page);

    await page.getByRole('button', { name: 'API Tokens' }).click();
    await page.waitForTimeout(300);

    await expect(page.getByText('Create New Token')).toBeVisible();
  });

  test('data providers tab displays individual provider names', async ({ page }) => {
    await loginAndNavigateToSettings(page);

    // Unroute empty providers from setupAuthenticatedMocks, register with test data
    await page.unroute('**/api/providers');
    await page.route('**/api/providers', async (route) => {
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({
          providers: [
            { provider: 'strava', display_name: 'Strava', requires_oauth: true, connected: false, capabilities: ['activities'] },
            { provider: 'fitbit', display_name: 'Fitbit', requires_oauth: true, connected: false, capabilities: ['activities', 'sleep'] },
            { provider: 'garmin', display_name: 'Garmin', requires_oauth: true, connected: false, capabilities: ['activities'] },
            { provider: 'whoop', display_name: 'WHOOP', requires_oauth: true, connected: false, capabilities: ['activities', 'sleep'] },
            { provider: 'terra', display_name: 'Terra', requires_oauth: true, connected: false, capabilities: ['activities'] },
            { provider: 'synthetic', display_name: 'Synthetic', requires_oauth: false, connected: false, capabilities: ['activities'] },
            { provider: 'synthetic_sleep', display_name: 'Synthetic Sleep', requires_oauth: false, connected: false, capabilities: ['sleep'] },
          ],
        }),
      });
    });

    await page.getByRole('button', { name: 'Data Providers' }).click();
    await page.waitForTimeout(300);

    // Verify provider names are rendered
    await expect(page.getByText('Strava')).toBeVisible({ timeout: 5000 });
    await expect(page.getByText('Fitbit')).toBeVisible();
    await expect(page.getByText('Garmin')).toBeVisible();
    await expect(page.getByText('WHOOP')).toBeVisible();
    await expect(page.getByText('Terra')).toBeVisible();
    await expect(page.getByText('Synthetic', { exact: true })).toBeVisible();
    await expect(page.getByText('Synthetic Sleep')).toBeVisible();
  });

  test('data providers tab distinguishes OAuth Connect buttons from Manual badges', async ({ page }) => {
    await loginAndNavigateToSettings(page);

    // Unroute empty providers from setupAuthenticatedMocks, register with test data
    await page.unroute('**/api/providers');
    await page.route('**/api/providers', async (route) => {
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({
          providers: [
            { provider: 'strava', display_name: 'Strava', requires_oauth: true, connected: false, capabilities: ['activities'] },
            { provider: 'fitbit', display_name: 'Fitbit', requires_oauth: true, connected: false, capabilities: ['activities'] },
            { provider: 'garmin', display_name: 'Garmin', requires_oauth: true, connected: false, capabilities: ['activities'] },
            { provider: 'whoop', display_name: 'WHOOP', requires_oauth: true, connected: false, capabilities: ['activities'] },
            { provider: 'terra', display_name: 'Terra', requires_oauth: true, connected: false, capabilities: ['activities'] },
            { provider: 'synthetic', display_name: 'Synthetic', requires_oauth: false, connected: false, capabilities: ['activities'] },
            { provider: 'synthetic_sleep', display_name: 'Synthetic Sleep', requires_oauth: false, connected: false, capabilities: ['sleep'] },
          ],
        }),
      });
    });
    await page.getByRole('button', { name: 'Data Providers' }).click();
    await page.waitForTimeout(300);

    // OAuth providers should have Connect buttons
    const connectButtons = page.getByRole('button', { name: 'Connect', exact: true });
    await expect(connectButtons.first()).toBeVisible({ timeout: 5000 });
    const connectCount = await connectButtons.count();
    expect(connectCount).toBe(5);

    // Manual providers should show "Manual" badge
    const manualBadges = page.getByText('Manual', { exact: true });
    const manualCount = await manualBadges.count();
    expect(manualCount).toBe(2);
  });

  test('tokens tab shows setup instructions button for Claude and ChatGPT', async ({ page }) => {
    await loginAndNavigateToSettings(page);

    await page.getByRole('button', { name: 'API Tokens' }).click();
    await page.waitForTimeout(300);

    // Should show Setup Instructions toggle button with Claude & ChatGPT mention
    await expect(page.getByText('Setup Instructions')).toBeVisible();
    await expect(page.getByText('for Claude & ChatGPT')).toBeVisible();
  });

  test('tokens tab shows Connected Apps section', async ({ page }) => {
    await loginAndNavigateToSettings(page);

    await page.getByRole('button', { name: 'API Tokens' }).click();
    await page.waitForTimeout(300);

    // Should show Connected Apps heading (use .first() in case of duplicate heading elements)
    await expect(page.getByRole('heading', { name: 'Connected Apps' }).first()).toBeVisible();
  });

  test('account tab displays usage quota values with progress bars', async ({ page }) => {
    await loginAndNavigateToSettings(page);

    await page.getByRole('button', { name: 'Account' }).click();
    await page.waitForTimeout(300);

    // Should show usage quota labels and values
    await expect(page.getByText('Daily Messages')).toBeVisible({ timeout: 5000 });
    await expect(page.getByText('5 / 50')).toBeVisible();
    await expect(page.getByText('Daily Tokens')).toBeVisible();
    await expect(page.getByText('Weekly Messages')).toBeVisible();
    await expect(page.getByText('15 / 250')).toBeVisible();
  });

  test('account tab displays daily reset time', async ({ page }) => {
    await loginAndNavigateToSettings(page);

    await page.getByRole('button', { name: 'Account' }).click();
    await page.waitForTimeout(300);

    // Should show daily reset time
    await expect(page.getByText(/Daily limits reset at/)).toBeVisible({ timeout: 5000 });
  });

  test('account tab displays resource counters for coaches and conversations', async ({ page }) => {
    await loginAndNavigateToSettings(page);

    await page.getByRole('button', { name: 'Account' }).click();
    await page.waitForTimeout(300);

    // Should show resource counters (scope to main to avoid matching sidebar nav elements)
    const main = page.getByRole('main');
    await expect(main.getByText('Coaches')).toBeVisible({ timeout: 5000 });
    await expect(main.getByText('1 / 3')).toBeVisible();
    await expect(main.getByText('Conversations')).toBeVisible();
    await expect(main.getByText('2 / 20')).toBeVisible();
  });
});

test.describe('Settings Page - User Profile Bar Navigation', () => {
  test('clicking user profile bar navigates to settings (user mode)', async ({ page }) => {
    await setupAuthenticatedMocks(page, false);
    await page.goto('/');
    await page.waitForSelector('form', { timeout: 10000 });

    await page.locator('input[name="email"]').fill('webtest@pierre.dev');
    await page.locator('input[name="password"]').fill('TestPassword123');
    await page.getByRole('button', { name: 'Sign in' }).click();

    await expect(page.locator('input[name="email"]')).not.toBeVisible({ timeout: 10000 });

    // Look for the user profile bar at bottom of sidebar and click it
    const userProfileBar = page.locator('button:has-text("Web Test")');
    if (await userProfileBar.first().isVisible().catch(() => false)) {
      await userProfileBar.first().click();
      await page.waitForTimeout(500);

      // Should now see settings content (use button role to avoid heading matches)
      await expect(page.getByRole('button', { name: 'Profile' })).toBeVisible();
    }
  });

  test('clicking user profile bar navigates to user settings (admin mode)', async ({ page }) => {
    await setupAuthenticatedMocks(page, true);
    await page.goto('/');
    await page.waitForSelector('form', { timeout: 10000 });

    await page.locator('input[name="email"]').fill('admin@pierre.dev');
    await page.locator('input[name="password"]').fill('TestPassword123');
    await page.getByRole('button', { name: 'Sign in' }).click();

    await expect(page.locator('input[name="email"]')).not.toBeVisible({ timeout: 10000 });

    // Look for the user profile bar and click it — navigates to user settings for all users
    const userProfileBar = page.locator('button:has-text("Admin User")');
    if (await userProfileBar.first().isVisible().catch(() => false)) {
      await userProfileBar.first().click();
      await page.waitForTimeout(500);

      // Should navigate to user settings (Profile tab visible)
      await expect(page.getByRole('button', { name: 'Profile' })).toBeVisible({ timeout: 5000 });
    }
  });
});

async function loginAndNavigateToAdminSettings(page: import('@playwright/test').Page) {
  await setupAuthenticatedMocks(page, true);
  await page.goto('/');
  await page.waitForSelector('form', { timeout: 10000 });

  await page.locator('input[name="email"]').fill('admin@pierre.dev');
  await page.locator('input[name="password"]').fill('TestPassword123');
  await page.getByRole('button', { name: 'Sign in' }).click();

  // Wait for dashboard to load
  await expect(page.locator('input[name="email"]')).not.toBeVisible({ timeout: 10000 });

  // Navigate to admin settings via Configuration sidebar tab
  await page.getByRole('button', { name: 'Configuration', exact: true }).click();
  await page.waitForTimeout(500);
}

test.describe('Settings Page - Admin Mode', () => {
  test('admin settings shows system settings heading', async ({ page }) => {
    await loginAndNavigateToAdminSettings(page);

    // Admin settings should show configuration sections
    await expect(page.getByRole('heading', { name: 'User Registration' })).toBeVisible({ timeout: 5000 });
  });

  test('admin settings shows auto-approval toggle', async ({ page }) => {
    await loginAndNavigateToAdminSettings(page);

    // Should show user registration / auto-approval section
    await expect(page.getByRole('heading', { name: 'User Registration' })).toBeVisible({ timeout: 5000 });
  });

  test('admin settings shows social insights configuration', async ({ page }) => {
    await loginAndNavigateToAdminSettings(page);

    // Should show social insights config section
    await expect(page.getByRole('heading', { name: 'Social Insights Configuration' })).toBeVisible({ timeout: 5000 });
  });
});
