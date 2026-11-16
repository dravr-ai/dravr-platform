// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: E2E for the Garmin connect (carnet#1472): a grant whose history is missing chains straight into the Garmin sign-in, in both themes
// ABOUTME: Nothing to choose: no "Bring in your history" sheet, and the card's Add history opens the same sign-in in one tap

import { test, expect, type Page, type Route } from '@playwright/test';
import { APP_SHELL_TIMEOUT_MS, setupDashboardMocks, loginToDashboard } from './test-helpers';

const json = (route: Route, body: unknown, status = 200) =>
  route.fulfill({ status, contentType: 'application/json', body: JSON.stringify(body) });

/** The Garmin card right after its grant: connected through Garmin, its history still to copy. */
const garminCard = {
  provider: 'sciotte_garmin',
  display_name: 'Garmin',
  description: 'Activities and health metrics from Garmin devices',
  requires_oauth: false,
  connected: true,
  connected_backend: 'garmin',
  connection_type: 'oauth',
  needs_reauth: false,
  capabilities: ['activities', 'sleep', 'health'],
  consent_required: false,
  recommended_backend: 'oauth',
  history: { state: 'needed', sign_in_target: 'sciotte_garmin', archive_source: 'garmin_archive' },
};

async function mockCards(page: Page) {
  await page.route('**/api/providers', (route) => json(route, { providers: [garminCard] }));
  await page.route('**/api/imports', (route) =>
    json(route, { enabled: true, sources: ['strava_archive', 'garmin_archive'], imports: [] }),
  );
  await page.route('**/api/imports/garmin_archive/current', (route) => json(route, { import: null }));
}

/**
 * Come back from a granted Garmin authorization the way the server does: the
 * connect started on the connections route, and the provider returns to the
 * app root with the result, a page load of its own. `settleOAuthReturn` stores
 * the result and puts the route back before the first render, so Settings is
 * the first surface to read it, as it is for an athlete.
 *
 * Never leave the result by hand on Home and then change the hash: Home's
 * thread reads and removes the same key when it mounts, and a mount that lands
 * after the write takes the grant before Settings opens.
 */
async function returnFromGarminGrant(page: Page) {
  // The return is a page load of its own: the session is restored on it the
  // way every reload restores one.
  await page.route('**/api/auth/session', (route) =>
    json(route, {
      access_token: 'test-jwt-token',
      csrf_token: 'test-csrf-token',
      user: {
        id: 'user-123',
        user_id: 'user-123',
        email: 'alice@acme.com',
        display_name: 'Alice Test',
        role: 'user',
        is_admin: false,
        user_status: 'active',
        tier: 'professional',
        tenant_id: 'user-123',
      },
    }),
  );
  await page.goto('/#settings/connections');
  await expect(page.getByTestId('provider-row-sciotte_garmin')).toBeVisible({ timeout: 10000 });
  // What `startOAuthInThisTab` keeps: the route the connect started from.
  await page.evaluate(() => sessionStorage.setItem('pierre_oauth_return', 'settings/connections'));
  await page.goto('/?provider=garmin&success=true');
  await page.waitForURL((url) => url.hash === '#settings/connections' && url.search === '', { timeout: APP_SHELL_TIMEOUT_MS });
}

async function expectHistorySignIn(page: Page) {
  const login = page.getByRole('dialog', { name: /Garmin/ });
  await expect(login).toBeVisible({ timeout: 10000 });
  await expect(login.getByTestId('sciotte-history-intro')).toHaveText(
    'Sign in to bring in your Garmin history. Your sign-in keeps new workouts coming too.',
  );
  // Nothing to choose, and no service name behind the sign-in.
  await expect(page.getByRole('dialog', { name: 'Bring in your Garmin history' })).toHaveCount(0);
  await expect(login.getByText(/sciotte/i)).toHaveCount(0);
  return login;
}

test.describe('Garmin grant chains into the sign-in (carnet#1472)', () => {
  test.beforeEach(async ({ page }) => {
    await setupDashboardMocks(page, { role: 'user', email: 'alice@acme.com', displayName: 'Alice Test' });
    await mockCards(page);
  });

  for (const theme of ['light', 'dark'] as const) {
    test(`a grant with its history missing opens the Garmin sign-in at once, ${theme} theme`, async ({ page }, testInfo) => {
      await loginToDashboard(page);
      await returnFromGarminGrant(page);
      await page.evaluate((t) => document.documentElement.classList.toggle('dark', t === 'dark'), theme);

      const login = await expectHistorySignIn(page);

      await page.waitForTimeout(500);
      await login.screenshot({ path: testInfo.outputPath(`garmin-grant-history-sign-in-${theme}.png`) });
    });
  }

  test("the card's Add history opens the same sign-in in one tap", async ({ page }) => {
    await loginToDashboard(page);
    await page.goto('/#settings/connections');
    const row = page.getByTestId('provider-row-sciotte_garmin');
    await expect(row).toBeVisible({ timeout: 10000 });

    await row.getByRole('button', { name: 'Add history' }).click();

    await expectHistorySignIn(page);
  });
});
