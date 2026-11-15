// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: E2E for the Garmin connect (carnet#1472): a grant whose history is missing chains straight into the Garmin sign-in, in both themes
// ABOUTME: Nothing to choose: no "Bring in your history" sheet, and the card's Add history opens the same sign-in in one tap

import { test, expect, type Page, type Route } from '@playwright/test';
import { setupDashboardMocks, loginToDashboard } from './test-helpers';

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
 * Leave the result the server's return from a granted Garmin authorization
 * leaves (`settleOAuthReturn`), for the surface the athlete lands on.
 */
async function leaveGarminGrant(page: Page) {
  await page.evaluate(() => {
    localStorage.setItem(
      'pierre_oauth_result',
      JSON.stringify({
        type: 'oauth_completed',
        provider: 'garmin',
        success: true,
        hasError: false,
        tombstoned: false,
        timestamp: Date.now(),
      }),
    );
  });
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
      await leaveGarminGrant(page);
      await page.goto('/#settings/connections');
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
