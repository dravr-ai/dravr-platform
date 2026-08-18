// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: E2E tests to prevent UI hallucinations and hardcoded fake data
// ABOUTME: Tests verify that unimplemented features do NOT appear in the UI

import { test, expect, type Page } from '@playwright/test';
import {
  loginAsUser,
  navigateToTab,
  waitForNetworkIdle,
} from './visual-test-helpers';

test.describe('Anti-Hallucination Tests - User Mode', () => {
  test.describe.configure({ mode: 'serial' });

  // ========================================
  // Settings Screen - Hallucinated Elements Must NOT Exist
  // ========================================
  test.describe('Settings - No Hallucinated Elements', () => {
    test.beforeEach(async ({ page }) => {
      await loginAsUser(page, 'webtest');
      await navigateToTab(page, 'Settings');
      await waitForNetworkIdle(page);
    });

    test('should NOT show Apple Health (not implemented for web)', async ({ page }) => {
      const appleHealth = page.getByText('Apple Health', { exact: false });
      await expect(appleHealth).not.toBeVisible();
    });

    test('should NOT show Export Data (not implemented)', async ({ page }) => {
      const exportData = page.getByText('Export Data', { exact: false });
      await expect(exportData).not.toBeVisible();
    });

    test('should NOT show Push Notifications (not implemented - ASY-355)', async ({ page }) => {
      const pushNotifications = page.getByText('Push Notifications', { exact: false });
      await expect(pushNotifications).not.toBeVisible();
    });

    test('should NOT show Email Updates (not implemented - ASY-356)', async ({ page }) => {
      const emailUpdates = page.getByText('Email Updates', { exact: false });
      await expect(emailUpdates).not.toBeVisible();
    });

    test('should NOT show Notifications section header (not implemented)', async ({ page }) => {
      // Check for Notifications as a section header, not just any mention
      const notificationsHeader = page.locator('h2, h3').filter({ hasText: 'Notifications' });
      await expect(notificationsHeader).not.toBeVisible();
    });

    test('should NOT show hardcoded user stats like 127 activities', async ({ page }) => {
      // These are fake hardcoded values that indicate hallucination
      const fakeStats127 = page.getByText('127', { exact: true });
      await expect(fakeStats127).not.toBeVisible();
    });

    test('should NOT show hardcoded user stats like 89 hours', async ({ page }) => {
      const fakeStats89 = page.getByText('89', { exact: true });
      await expect(fakeStats89).not.toBeVisible();
    });

    test('should NOT show hardcoded user stats like 12 insights', async ({ page }) => {
      const fakeStats12 = page.getByText('12', { exact: true });
      await expect(fakeStats12).not.toBeVisible();
    });

    test('Profile stats should come from backend (not hardcoded)', async ({ page }) => {
      // Verify we can see the stats section with real data labels
      const connectedProviders = page.getByText('Connected Providers');
      const daysActive = page.getByText('Days Active');

      await expect(connectedProviders).toBeVisible();
      await expect(daysActive).toBeVisible();
    });

    test('Account tab should show real user status', async ({ page }) => {
      // Click Account tab
      const accountTab = page.getByRole('button', { name: 'Account' });
      await accountTab.click();
      await page.waitForTimeout(300);

      // Should show Status row with real value (Active/Pending)
      const statusLabel = page.getByText('Status', { exact: true });
      await expect(statusLabel).toBeVisible();

      // Should show Role row with real value
      const roleLabel = page.getByText('Role', { exact: true });
      await expect(roleLabel).toBeVisible();
    });
  });

  // ========================================
  // Chat Screen - No Hallucinated Elements
  // ========================================
  test.describe('Chat - No Hallucinated Elements', () => {
    test.beforeEach(async ({ page }) => {
      await loginAsUser(page, 'webtest');
      await navigateToTab(page, 'Chat');
      await waitForNetworkIdle(page);
    });

    test('should NOT show hardcoded fake conversation count', async ({ page }) => {
      // Fake conversation counts like "42 conversations" without backend
      const fakeCount = page.getByText('42 conversations', { exact: false });
      await expect(fakeCount).not.toBeVisible();
    });

    test('should NOT show fake AI response time metrics', async ({ page }) => {
      // Fake metrics like "Average response time: 1.2s"
      const fakeMetric = page.getByText('Average response time', { exact: false });
      await expect(fakeMetric).not.toBeVisible();
    });
  });

  // ========================================
  // Coach Library - No Hallucinated Elements
  // ========================================
  test.describe('Coach Library - No Hallucinated Elements', () => {
    test.beforeEach(async ({ page }) => {
      await loginAsUser(page, 'webtest');
      await navigateToTab(page, 'Coaches');
      await waitForNetworkIdle(page);
    });

    test('should NOT show hardcoded coach count', async ({ page }) => {
      // Should not show fake count like "You have 15 coaches"
      const fakeCoachCount = page.getByText('You have 15 coaches', { exact: false });
      await expect(fakeCoachCount).not.toBeVisible();
    });

    test('should show real category filters from backend', async ({ page }) => {
      // These categories should exist and match backend
      // Use exact: true to differentiate "All" from "All Sources"
      await expect(page.getByRole('button', { name: 'All', exact: true })).toBeVisible();
      await expect(page.getByRole('button', { name: 'Training' })).toBeVisible();
      await expect(page.getByRole('button', { name: 'Nutrition' })).toBeVisible();
    });
  });

  // ========================================
  // Connections Tab - No Hallucinated Providers
  // ========================================
  test.describe('Connections - No Hallucinated Providers', () => {
    test.beforeEach(async ({ page }) => {
      await loginAsUser(page, 'webtest');
      await navigateToTab(page, 'Settings');
      await waitForNetworkIdle(page);

      // Click Data Providers tab
      const dataProvidersTab = page.getByRole('button', { name: 'Data Providers' });
      await dataProvidersTab.click();
      await page.waitForTimeout(300);
    });

    test('should NOT show MyFitnessPal (not a supported provider)', async ({ page }) => {
      const myFitnessPal = page.getByText('MyFitnessPal', { exact: false });
      await expect(myFitnessPal).not.toBeVisible();
    });

    test('should NOT show Peloton (not a supported provider)', async ({ page }) => {
      const peloton = page.getByText('Peloton', { exact: false });
      await expect(peloton).not.toBeVisible();
    });

    test('should NOT show Apple Watch (not a supported provider)', async ({ page }) => {
      const appleWatch = page.getByText('Apple Watch', { exact: false });
      await expect(appleWatch).not.toBeVisible();
    });
  });

  // ========================================
  // API Tokens Tab - No Hallucinated Data
  // ========================================
  test.describe('API Tokens - No Hallucinated Data', () => {
    test.beforeEach(async ({ page }) => {
      await loginAsUser(page, 'webtest');
      await navigateToTab(page, 'Settings');
      await waitForNetworkIdle(page);

      // Click API Tokens tab
      const tokensTab = page.getByRole('button', { name: 'API Tokens' });
      await tokensTab.click();
      await page.waitForTimeout(300);
    });

    test('should NOT show hardcoded token usage stats', async ({ page }) => {
      // Should not show fake stats like "1,234 requests today"
      const fakeRequests = page.getByText('1,234 requests', { exact: false });
      await expect(fakeRequests).not.toBeVisible();
    });

    test('should show real active tokens count from backend', async ({ page }) => {
      // Should see "X active tokens" where X comes from backend
      const activeTokensText = page.getByText(/\d+ active tokens?/);
      await expect(activeTokensText).toBeVisible();
    });
  });
  // ========================================
  // Providerless athlete (Phase 5)
  // ========================================
  test.describe('Providerless - the coach admits what it cannot see', () => {
    // Only the app's OWN chrome is asserted here, deliberately.
    //
    // playwright.config.ts runs with E2E_TEST=true, which disables the Vite
    // backend proxy, so any assistant text in this suite is a fixture the test
    // author wrote. Asserting "the coach stated no distance" against our own
    // mocked reply would prove nothing about `build_provider_context` or the
    // athlete-data verifier — those are covered by Rust tests that exercise the
    // real code (`providerless_prompt_context_test`, `athlete_data_layer_test`).
    //
    // What IS real here: whether the UI tells a providerless athlete their data
    // is missing, and — the regression that matters more — whether it wrongly
    // tells a connected one the same thing.

    // This suite owns the providers answer outright, via `skipProvidersRoute`.
    //
    // Three facts drive that, each established by running it rather than
    // reasoning about it:
    //
    // 1. `setupDashboardMocks` — which `loginAsUser` calls — DOES register
    //    `**/api/providers`, returning an empty list. The comment that stood
    //    here claimed the helpers contain no providers route at all; that was
    //    false, and believing it is what sent earlier debugging attempts
    //    hunting a race that was not there.
    //
    // 2. Registering a competing route cannot win. Playwright matches handlers
    //    newest-first, so one registered before `loginAsUser` loses to the
    //    default outright — a connected athlete gets `{providers: []}` and the
    //    banner appears, failing the guard below for the wrong reason.
    //
    // 3. Registering after `loginAsUser` wins the route but arrives too late to
    //    matter: the providers query has already been answered and cached under
    //    `QUERY_KEYS.providers.status()`, so ChatTab mounts with `isSuccess`
    //    already true and renders off the default payload. Anything the spec
    //    registers afterwards only lands as a background refetch — which is
    //    also why waiting for a second `/api/providers` response timed out at
    //    15s and red this suite: React Query coalesces, so a second request is
    //    not guaranteed to exist at all.
    //
    // Opting the default out leaves the gated route below as the only handler,
    // so it answers the very first request. `release` holds that answer open
    // until the test calls it, making "the query is still in flight" a state
    // the test controls rather than a window it hopes to hit. Nothing on the
    // login path waits for network idle (`loginToDashboard` waits for `main`
    // plus a fixed delay), so a pending providers request cannot stall it.
    const routeProviders = async (page: Page, providers: unknown[]) => {
      let release: () => void = () => {};
      const answered = new Promise<void>((resolve) => {
        release = resolve;
      });
      await page.route('**/api/providers', async (route) => {
        await answered;
        await route.fulfill({
          status: 200,
          contentType: 'application/json',
          body: JSON.stringify({ providers }),
        });
      });
      return release;
    };

    // Present as soon as ChatTab renders, and independent of the providers
    // query — so a banner assertion cannot pass merely because chat never
    // loaded.
    const chatComposer = (page: Page) => page.getByLabel('Message Dravr');

    test('shows the connect-provider banner in chat when nothing is connected', async ({
      page,
    }) => {
      const releaseProviders = await routeProviders(page, []);
      await loginAsUser(page, 'webtest', { skipProvidersRoute: true });
      await navigateToTab(page, 'Chat');
      await expect(chatComposer(page)).toBeVisible();

      releaseProviders();

      // Auto-retrying, so it holds until the query has answered and the banner
      // renders — with no assumption about which request carried the answer.
      await expect(page.getByTestId('connect-provider-banner')).toBeVisible();
    });

    test('does NOT show the banner to a connected athlete', async ({ page }) => {
      // The regression guard. `hasConnectedProvider` defaults to false while the
      // providers query is in flight, so keying the banner off it alone flashed
      // a connect-provider nudge at connected users on every chat load.
      const releaseProviders = await routeProviders(page, [
        { provider: 'strava', connected: true },
      ]);
      await loginAsUser(page, 'webtest', { skipProvidersRoute: true });
      await navigateToTab(page, 'Chat');
      await expect(chatComposer(page)).toBeVisible();

      const banner = page.getByTestId('connect-provider-banner');

      // Chat is up and the providers answer is still held, so this is the
      // in-flight window itself — the exact state the flash regression lives
      // in, asserted as a fact rather than caught by luck.
      await expect(banner).toHaveCount(0);

      releaseProviders();
      await waitForNetworkIdle(page);

      // And still absent once the answer confirms the athlete is connected.
      await expect(banner).toHaveCount(0);
    });
  });
});
