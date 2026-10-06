// ABOUTME: Playwright E2E tests for the Settings page UX redesign.
// ABOUTME: Tests user settings tabs, change password modal, about tab, and admin settings navigation.

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { test, expect } from '@playwright/test';
import { applyTestStubs, signInThroughHostedPage, SIGN_IN_BUTTON } from './test-helpers';

// Helper to set up mocks for an authenticated user session
interface MockOptions {
  providers?: Array<{ provider: string; display_name: string; description?: string; requires_oauth: boolean; connected: boolean; capabilities: string[] }>;
  onboardingSteps?: Array<{ step_id: string; status: 'complete' | 'skipped' }>;
}

async function setupAuthenticatedMocks(page: import('@playwright/test').Page, isAdmin = false, options: MockOptions = {}) {
  await applyTestStubs(page);
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

  // Mock pending users (for admin)
  await page.route('**/api/admin/users/pending', async (route) => {
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify([]),
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

  // Mock A2A clients list (used by API Tokens tab)
  await page.route('**/a2a/clients', async (route) => {
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify([]),
    });
  });

  // Mock LLM settings (read by the About pane for the model line)
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
      body: JSON.stringify({ data: { enabled: false, auto_approve_domains: [], overridden_by_env: false, description: 'Auto-approve new users' } }),
    });
  });

  // Mock providers status (needed by ChatTab and ProviderConnectionCards)
  await page.route('**/api/providers', async (route) => {
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({ providers: options.providers ?? [] }),
    });
  });

  // Mock coaches (the chat header and the @handle palette read this list)
  await page.route('**/api/agents**', async (route) => {
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({ agents: [], total: 0, metadata: { timestamp: new Date().toISOString(), api_version: 'v1' } }),
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

  // Mock store endpoints (needed by Discover tab)
  await page.route('**/api/store/**', async (route) => {
    const url = route.request().url();
    if (url.includes('/installations')) {
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({ agents: [], metadata: { timestamp: new Date().toISOString(), api_version: 'v1' } }),
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
        body: JSON.stringify({ agents: [], next_cursor: null, has_more: false, metadata: { timestamp: new Date().toISOString(), api_version: 'v1' } }),
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

  // Mock user LLM settings (read by the About pane for the model line)
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
          system_provider: {
            name: 'copilot_headless',
            display_name: 'Copilot Headless',
            model: 'claude-sonnet-5',
          },
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
      body: JSON.stringify({ success: true, message: 'Retrieved 0 pending users', data: { count: 0, users: [] } }),
    });
  });

  // Mock admin users list
  await page.route('**/api/admin/users**', async (route) => {
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({ success: true, message: 'Retrieved users', data: { users: [], total: 0, has_more: false } }),
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
        resources: { agents: 1, max_agents: 3, conversations: 2, max_conversations: 20 },
      }),
    });
  });

  // The durable onboarding record, for a spec whose surface the onboarding
  // steps would otherwise intercept (the chat-app step reads the same channel
  // list the Messaging pane does). Registered last so it wins over the stub.
  if (options.onboardingSteps) {
    const steps = options.onboardingSteps;
    await page.route('**/api/me/onboarding-status', async (route) => {
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({ needs_provider_connection: false, steps }),
      });
    });
  }
}

async function loginAndNavigateToSettings(
  page: import('@playwright/test').Page,
  isAdmin = false,
  options: MockOptions = {}
) {
  await setupAuthenticatedMocks(page, isAdmin, options);
  await page.goto('/');
  await signInThroughHostedPage(page);

  // Wait for dashboard to load
  await expect(page.locator(SIGN_IN_BUTTON)).not.toBeVisible({ timeout: 10000 });

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
    await expect(page.getByRole('button', { name: 'MCP tokens' })).toBeVisible();
    // No AI Settings pane: nobody brings their own model, and the pane stored a
    // key while changing nothing about the coaching that followed.
    await expect(page.getByRole('button', { name: 'AI Settings' })).toHaveCount(0);
    await expect(page.getByRole('button', { name: 'About' })).toBeVisible();
    await expect(page.getByRole('button', { name: 'Account' })).toBeVisible();
  });

  test('profile tab shows user info and stats', async ({ page }) => {
    await loginAndNavigateToSettings(page);

    // The name heads the settings menu too, so the assertion targets the profile pane
    await expect(page.getByTestId('settings-pane').getByText('Web Test')).toBeVisible();
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

    // The menu row's hint names "Version" too, so the assertions read the open pane
    const pane = page.getByTestId('settings-pane');
    await expect(pane.getByText('Version')).toBeVisible();
    await expect(pane.getByText('1.0.0')).toBeVisible();
    await expect(pane.getByText('Help Center')).toBeVisible();
    // The legal row is named as a DOCUMENT: "Terms & Privacy" read one word
    // from the "Privacy & Data" pane and went somewhere else entirely.
    await expect(pane.getByText('Legal documents')).toBeVisible();
    await expect(pane.getByText('Terms & Privacy')).toHaveCount(0);
    // Which model answers, read-only: a fact about the product, not a field
    // that invites a credential.
    await expect(pane.getByTestId('about-coach-model-value')).toBeVisible();
    // Both external rows go to a page that answers: /help is still a 404 so
    // help goes to the docs hub; /privacy went live on 2026-09-18.
    await expect(pane.getByTestId('about-section-help')).toHaveAttribute(
      'href',
      'https://dravr.ai/docs',
    );
    await expect(pane.getByTestId('about-section-legal')).toHaveAttribute(
      'href',
      'https://dravr.ai/privacy',
    );
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

    await page.getByRole('button', { name: 'MCP tokens' }).click();
    await page.waitForTimeout(300);

    await expect(page.getByText('Create New Token')).toBeVisible();
  });

  test('API keys tab shows a created key once and a revoked key leaves', async ({ page }) => {
    await loginAndNavigateToSettings(page);
    let keys = [
      {
        id: 'k1', name: 'Export script', description: null, tier: 'starter', key_prefix: 'pk_live_k1',
        is_active: true, last_used_at: null, expires_at: null, created_at: '2026-09-20T08:30:00Z',
      },
    ];
    await page.route('**/api/keys', async (route) => {
      if (route.request().method() === 'POST') {
        const created = { ...keys[0], id: 'k2', name: 'Garmin sync', key_prefix: 'pk_live_k2' };
        keys = [...keys, created];
        await route.fulfill({
          status: 201,
          contentType: 'application/json',
          body: JSON.stringify({ api_key: 'pk_live_k2_full_secret', key_info: created, warning: 'Store it.' }),
        });
        return;
      }
      await route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify({ api_keys: keys }) });
    });
    await page.route('**/api/keys/*/usage*', (route) =>
      route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({ stats: { api_key_id: 'k1', total_requests: 3, successful_requests: 3, failed_requests: 0, total_response_time_ms: 10, tool_usage: {}, period_start: '', period_end: '' } }),
      })
    );
    await page.route('**/api/keys/k1', async (route) => {
      keys = keys.map((k) => (k.id === 'k1' ? { ...k, is_active: false } : k));
      await route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify({ message: 'ok', deactivated_at: '2026-09-28T10:00:00Z' }) });
    });

    await page.getByRole('button', { name: 'API keys' }).click();
    await expect(page.getByTestId('api-key-k1')).toContainText('Export script');
    await expect(page.getByTestId('api-key-usage-k1')).toHaveText('3 requests in the last 30 days');

    await page.getByTestId('api-key-name').fill('Garmin sync');
    await page.getByTestId('api-key-create').click();
    await expect(page.getByTestId('api-key-secret')).toHaveText('pk_live_k2_full_secret');
    await page.getByTestId('api-key-done').click();
    await expect(page.getByTestId('api-key-secret')).toHaveCount(0);
    await expect(page.getByTestId('api-key-k2')).toContainText('Garmin sync');

    await page.getByTestId('api-key-revoke-k1').click();
    await page.getByRole('dialog').getByRole('button', { name: 'Revoke' }).click();
    await expect(page.getByTestId('api-key-k1')).toHaveCount(0);
  });

  test('data providers tab displays individual provider names', async ({ page }) => {
    // The API surfaces `sciotte` (Strava-branded), `sciotte_garmin`
    // (Garmin-branded), `sciotte_trainingpeaks` (TrainingPeaks-branded) and `whoop`.
    const testProviders = [
      { provider: 'sciotte', display_name: 'Strava', description: 'Running, cycling, and swimming activities', requires_oauth: false, connected: false, capabilities: ['activities'] },
      { provider: 'sciotte_garmin', display_name: 'Garmin', description: 'Activities and health metrics from Garmin devices', requires_oauth: false, connected: false, capabilities: ['activities', 'sleep', 'recovery', 'health'] },
      { provider: 'sciotte_trainingpeaks', display_name: 'TrainingPeaks', description: 'Completed workouts and their training load from TrainingPeaks', requires_oauth: false, connected: false, capabilities: ['activities'], consent_required: true },
      { provider: 'whoop', display_name: 'WHOOP', description: 'Recovery, strain, and sleep metrics', requires_oauth: true, connected: false, capabilities: ['activities', 'sleep'] },
    ];
    await loginAndNavigateToSettings(page, false, { providers: testProviders });

    await page.getByRole('button', { name: 'Data Providers' }).click();
    await page.waitForTimeout(300);

    // Verify provider names are rendered (exact match avoids description text collisions)
    await expect(page.getByText('Strava', { exact: true })).toBeVisible({ timeout: 5000 });
    await expect(page.getByText('Garmin', { exact: true })).toBeVisible();
    await expect(page.getByText('TrainingPeaks', { exact: true })).toBeVisible();
    await expect(page.getByText('WHOOP', { exact: true })).toBeVisible();
    // Each row's line under the name is the one the server serves, as served
    await expect(page.getByText('Activities and health metrics from Garmin devices')).toBeVisible();
    await expect(page.getByText('Recovery, strain, and sleep metrics')).toBeVisible();
  });

  test('TrainingPeaks connects only after its notice is accepted', async ({ page }) => {
    const testProviders = [
      { provider: 'sciotte_garmin', display_name: 'Garmin', requires_oauth: false, connected: false, capabilities: ['activities', 'sleep', 'recovery', 'health'], consent_required: false },
      { provider: 'sciotte_trainingpeaks', display_name: 'TrainingPeaks', requires_oauth: false, connected: false, capabilities: ['activities'], consent_required: true },
    ];
    await loginAndNavigateToSettings(page, false, { providers: testProviders });

    const logins: Array<Record<string, unknown>> = [];
    await page.route('**/api/providers/sciotte/config', (route) =>
      route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify({ login_timeout_secs: 240 }) })
    );
    await page.route('**/api/providers/sciotte/login', async (route) => {
      logins.push(route.request().postDataJSON());
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({ status: 'connected', provider: 'sciotte_trainingpeaks' }),
      });
    });

    await page.getByRole('button', { name: 'Data Providers' }).click();
    await page
      .getByTestId('provider-row-sciotte_trainingpeaks')
      .getByRole('button', { name: 'Connect', exact: true })
      .click();

    const dialog = page.getByRole('dialog');
    const notice = dialog.getByRole('note');
    await expect(notice).toContainText('Before you connect TrainingPeaks');
    await expect(notice).toContainText('Terms of Use (section 13)');
    // A coach's account also reads the athletes who confirm a link, so the
    // notice says so before the account is connected.
    await expect(notice).toContainText(
      "If yours is a human coach's account, Dravr also uses it to read the calendars of the athletes who confirm a link in a group you oversee.",
    );

    // TrainingPeaks signs in with a username, and nothing is sent until the
    // notice is accepted.
    await dialog.getByLabel('Username').fill('coach-account');
    await dialog.getByLabel('Password', { exact: true }).fill('not-a-real-password');
    const logIn = dialog.getByRole('button', { name: 'Log In' });
    await expect(logIn).toBeDisabled();

    await dialog
      .getByLabel('I understand that TrainingPeaks could suspend my account, and I accept that risk.')
      .check();
    await expect(logIn).toBeEnabled();
    await logIn.click();

    // Settings closes the modal as soon as the connection lands.
    await expect(dialog).toBeHidden();
    await expect.poll(() => logins.length).toBe(1);
    expect(logins[0]).toEqual({
      email: 'coach-account',
      password: 'not-a-real-password',
      method: 'email',
      target: 'trainingpeaks',
      tos_consent: true,
    });
  });

  test('COROS connects with an email only after its notice is accepted', async ({ page }) => {
    const testProviders = [
      { provider: 'sciotte_trainingpeaks', display_name: 'TrainingPeaks', requires_oauth: false, connected: false, capabilities: ['activities'], consent_required: false },
      { provider: 'sciotte_coros', display_name: 'COROS', requires_oauth: false, connected: false, capabilities: ['activities', 'recovery', 'health'], consent_required: true },
    ];
    await loginAndNavigateToSettings(page, false, { providers: testProviders });

    const logins: Array<Record<string, unknown>> = [];
    await page.route('**/api/providers/sciotte/config', (route) =>
      route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify({ login_timeout_secs: 240 }) })
    );
    await page.route('**/api/providers/sciotte/login', async (route) => {
      logins.push(route.request().postDataJSON());
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({ status: 'connected', provider: 'sciotte_coros' }),
      });
    });

    await page.getByRole('button', { name: 'Data Providers' }).click();
    await page
      .getByTestId('provider-row-sciotte_coros')
      .getByRole('button', { name: 'Connect', exact: true })
      .click();

    // COROS states its own notice, never the TrainingPeaks one.
    const dialog = page.getByRole('dialog');
    const notice = dialog.getByRole('note');
    await expect(notice).toContainText('Before you connect COROS');
    await expect(notice).toContainText('Terms of Service (sections 4 and 7)');
    await expect(notice).not.toContainText('TrainingPeaks');

    // COROS signs in with an email, and nothing is sent until the notice is
    // accepted.
    const email = dialog.getByLabel('Email');
    await expect(email).toHaveAttribute('type', 'email');
    await email.fill('athlete@example.com');
    await dialog.getByLabel('Password', { exact: true }).fill('not-a-real-password');
    const logIn = dialog.getByRole('button', { name: 'Log In' });
    await expect(logIn).toBeDisabled();

    await dialog
      .getByLabel('I understand that COROS could suspend my account, and I accept that risk.')
      .check();
    await expect(logIn).toBeEnabled();
    await logIn.click();

    await expect(dialog).toBeHidden();
    await expect.poll(() => logins.length).toBe(1);
    expect(logins[0]).toEqual({
      email: 'athlete@example.com',
      password: 'not-a-real-password',
      method: 'email',
      target: 'coros',
      tos_consent: true,
    });
  });

  test('a TrainingPeaks row read through a coach names the coach and unlinks', async ({ page }) => {
    const testProviders = [
      {
        provider: 'sciotte_trainingpeaks',
        display_name: 'TrainingPeaks',
        requires_oauth: false,
        connected: true,
        needs_reauth: false,
        capabilities: ['activities'],
        consent_required: false,
        delegation: {
          connection_id: 'dc-1',
          group_id: 'group-1',
          group_name: 'Marathon Squad',
          coach_display_name: 'Casey Coach',
          status: 'confirmed',
          coach_needs_reauth: false,
          read_refused: null,
        },
      },
      {
        provider: 'sciotte_garmin',
        display_name: 'Garmin',
        requires_oauth: false,
        connected: true,
        needs_reauth: false,
        capabilities: ['activities', 'sleep', 'recovery', 'health'],
        consent_required: false,
        account_role: 'coach',
      },
    ];
    await loginAndNavigateToSettings(page, false, { providers: testProviders });

    await page.getByRole('button', { name: 'Data Providers' }).click();

    await expect(page.getByTestId('provider-delegated-sciotte_trainingpeaks')).toHaveText(
      'Connected through Casey Coach',
    );
    await expect(page.getByTestId('provider-disconnect-sciotte_trainingpeaks')).toHaveText('Unlink');
    // A coach account is badged, with where its athletes are linked.
    await expect(page.getByTestId('provider-coach-account-sciotte_garmin')).toHaveText('Coach account');
    await expect(
      page.getByText('TrainingPeaks keeps no calendar for a coach account. Link your athletes from a group you coach.'),
    ).toBeVisible();
  });

  test('data providers tab shows Connect affordance for all surfaced providers', async ({ page }) => {
    // Every surfaced provider is connectable: sciotte / sciotte_garmin /
    // sciotte_trainingpeaks via credential login, whoop via OAuth.
    const testProviders = [
      { provider: 'sciotte', display_name: 'Strava', requires_oauth: false, connected: false, capabilities: ['activities'] },
      { provider: 'sciotte_garmin', display_name: 'Garmin', requires_oauth: false, connected: false, capabilities: ['activities', 'sleep', 'recovery', 'health'] },
      { provider: 'sciotte_trainingpeaks', display_name: 'TrainingPeaks', requires_oauth: false, connected: false, capabilities: ['activities'], consent_required: true },
      { provider: 'whoop', display_name: 'WHOOP', requires_oauth: true, connected: false, capabilities: ['activities'] },
    ];
    await loginAndNavigateToSettings(page, false, { providers: testProviders });

    await page.getByRole('button', { name: 'Data Providers' }).click();
    await page.waitForTimeout(300);

    // All four providers should have Connect buttons
    const connectButtons = page.getByRole('button', { name: 'Connect', exact: true });
    await expect(connectButtons.first()).toBeVisible({ timeout: 5000 });
    const connectCount = await connectButtons.count();
    expect(connectCount).toBe(4);
  });

  test('tokens tab shows setup instructions button for Claude and ChatGPT', async ({ page }) => {
    await loginAndNavigateToSettings(page);

    await page.getByRole('button', { name: 'MCP tokens' }).click();
    await page.waitForTimeout(300);

    // Should show Setup Instructions toggle button with Claude & ChatGPT mention
    await expect(page.getByText('Setup Instructions')).toBeVisible();
    await expect(page.getByText('for Claude & ChatGPT')).toBeVisible();
  });

  test('tokens tab shows Connected Apps section', async ({ page }) => {
    await loginAndNavigateToSettings(page);

    await page.getByRole('button', { name: 'MCP tokens' }).click();
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

  test('account tab displays resource counters for agents and conversations', async ({ page }) => {
    await loginAndNavigateToSettings(page);

    await page.getByRole('button', { name: 'Account' }).click();
    await page.waitForTimeout(300);

    // Should show resource counters (scope to main to avoid matching sidebar nav elements)
    const main = page.getByRole('main');
    await expect(main.getByText('Agents')).toBeVisible({ timeout: 5000 });
    await expect(main.getByText('1 / 3')).toBeVisible();
    await expect(main.getByText('Conversations')).toBeVisible();
    await expect(main.getByText('2 / 20')).toBeVisible();
  });

  test('messaging pane links Telegram via QR after onboarding, then unlinks it', async ({ page }) => {
    // The athlete skipped the chat-app steps during onboarding — the case the
    // pane exists for: nothing else on the web could link Telegram afterwards.
    await loginAndNavigateToSettings(page, false, {
      onboardingSteps: [
        { step_id: 'messaging_channel', status: 'skipped' },
        { step_id: 'messaging_configure', status: 'skipped' },
      ],
    });

    const state = { linked: false, deleted: 0 };
    const stub = (route: import('@playwright/test').Route, body: unknown) =>
      route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify(body) });
    await page.route('**/api/messaging/channels', (route) => stub(route, { tenant_id: 't-1', channels: [] }));
    await page.route('**/api/messaging/channels/available', (route) =>
      stub(route, [
        { channel: 'telegram', display_name: 'Telegram', method: 'deep_link', recommended: true },
        { channel: 'slack', display_name: 'Slack', method: 'oauth', recommended: false },
      ]),
    );
    await page.route('**/api/messaging/link/init/**', (route) =>
      stub(route, {
        channel: 'telegram',
        method: 'deep_link',
        code: 'abc123',
        linking_url: 'https://t.me/DravrBot?start=abc123',
        expires_at: '2030-01-01T00:00:00Z',
        qr_svg: '<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"></svg>',
      }),
    );
    await page.route('**/api/messaging/links', (route) =>
      stub(route, {
        links: state.linked
          ? [{ channel: 'telegram', channel_user_id: 'tg-42', display_name: '@webtest', linked_at: '2026-09-01T10:00:00Z' }]
          : [],
      }),
    );
    await page.route('**/api/messaging/links/*', (route) => {
      if (route.request().method() !== 'DELETE') return route.fallback();
      state.deleted += 1;
      state.linked = false;
      return route.fulfill({ status: 204, body: '' });
    });

    await page.getByTestId('settings-menu-messaging').click();

    const linkedSection = page.getByTestId('chat-app-linked-section');
    await expect(linkedSection.getByTestId('chat-app-no-links')).toHaveText(
      'No chat apps linked yet. Link one below to message your agent from it.',
    );
    await expect(page.getByTestId('chat-app-add-slack')).toBeVisible();

    await page.getByTestId('chat-app-connect-telegram').click();
    const panel = page.getByTestId('channel-link-panel');
    await expect(panel.getByRole('img', { name: 'QR code to connect Telegram' })).toBeVisible();
    await expect(panel.getByRole('link')).toHaveAttribute('href', 'https://t.me/DravrBot?start=abc123');

    // The athlete presses Start in Telegram; the pane's poll sees the link land.
    state.linked = true;
    await expect(panel).toBeHidden({ timeout: 10_000 });
    const row = page.getByTestId('chat-app-link-telegram');
    await expect(row).toContainText('Telegram');
    await expect(row).toContainText('@webtest');
    await expect(page.getByTestId('chat-app-add-telegram')).toHaveCount(0);

    await page.getByTestId('chat-app-unlink-telegram').click();
    const dialog = page.getByRole('dialog');
    await expect(dialog.getByText('Unlink Telegram?')).toBeVisible();
    await dialog.getByRole('button', { name: 'Unlink' }).click();

    await expect(page.getByTestId('chat-app-link-telegram')).toHaveCount(0);
    await expect(page.getByTestId('chat-app-connect-telegram')).toBeVisible();
    expect(state.deleted).toBe(1);
  });
});

test.describe('Settings Page - User Profile Bar Navigation', () => {
  test('clicking user profile bar navigates to settings (user mode)', async ({ page }) => {
    await setupAuthenticatedMocks(page, false);
    await page.goto('/');
    await signInThroughHostedPage(page);

    await expect(page.locator(SIGN_IN_BUTTON)).not.toBeVisible({ timeout: 10000 });

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
    await signInThroughHostedPage(page);

    await expect(page.locator(SIGN_IN_BUTTON)).not.toBeVisible({ timeout: 10000 });

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
  await signInThroughHostedPage(page);

  // Wait for dashboard to load
  await expect(page.locator(SIGN_IN_BUTTON)).not.toBeVisible({ timeout: 10000 });

  // Navigate to admin settings via Platform Settings sidebar tab
  await page.getByRole('button', { name: 'Platform Settings', exact: true }).click();
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

  test('admin settings carries no social insights configuration', async ({ page }) => {
    await loginAndNavigateToAdminSettings(page);

    // The social feed was retired by the Chat-First Cutover, and its tuning
    // card went with it: the page renders its remaining sections and nothing
    // named after insights.
    await expect(page.getByRole('heading', { name: 'User Registration' })).toBeVisible({ timeout: 5000 });
    await expect(page.getByRole('heading', { name: 'Social Insights Configuration' })).toHaveCount(0);
    await expect(page.getByText(/social insights/i)).toHaveCount(0);
  });
});
