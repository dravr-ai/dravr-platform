// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: E2E for the "Bring in your history" sheet (carnet#1454): a Garmin card's Add history opens it, in both themes
// ABOUTME: Its sign-in opens the Garmin login in history mode, its export opens the import window, and Not now closes it

import { test, expect, type Page, type Route } from '@playwright/test';
import { setupDashboardMocks, loginToDashboard } from './test-helpers';

const json = (route: Route, body: unknown, status = 200) =>
  route.fulfill({ status, contentType: 'application/json', body: JSON.stringify(body) });

/** The Garmin card after its grant, its history still to bring in; `archive_source` as the server offers it. */
function garminCard(archive_source: string | null) {
  return {
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
    history: { state: 'needed', sign_in_target: 'sciotte_garmin', archive_source },
  };
}

async function mockCards(page: Page, archive_source: string | null) {
  await page.route('**/api/providers', (route) => json(route, { providers: [garminCard(archive_source)] }));
  await page.route('**/api/imports', (route) =>
    json(route, { enabled: true, sources: ['strava_archive', 'garmin_archive'], imports: [] }),
  );
  await page.route('**/api/imports/garmin_archive/current', (route) => json(route, { import: null }));
  // The import window is a page load of its own: the session is restored on it.
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
}

async function openSheet(page: Page) {
  await loginToDashboard(page);
  await page.goto('/#settings/connections');
  const row = page.getByTestId('provider-row-sciotte_garmin');
  await expect(row).toBeVisible({ timeout: 10000 });
  await row.getByRole('button', { name: 'Add history' }).click();
  const sheet = page.getByRole('dialog', { name: 'Bring in your Garmin history' });
  await expect(sheet).toBeVisible();
  return sheet;
}

test.describe('Bring in your history sheet (carnet#1454)', () => {
  test.beforeEach(async ({ page }) => {
    await setupDashboardMocks(page, { role: 'user', email: 'alice@acme.com', displayName: 'Alice Test' });
  });

  for (const theme of ['light', 'dark'] as const) {
    test(`offers the sign-in, the export and Not now in the ${theme} theme`, async ({ page }, testInfo) => {
      await mockCards(page, 'garmin_archive');
      const sheet = await openSheet(page);
      await page.evaluate((t) => document.documentElement.classList.toggle('dark', t === 'dark'), theme);

      await expect(sheet).toContainText('Garmin sends what you record from today.');
      await expect(sheet.getByRole('button', { name: /Sign in once to copy your past workouts/ })).toBeVisible();
      await expect(sheet.getByRole('button', { name: /Import your Garmin export/ })).toBeVisible();
      await expect(sheet.getByRole('button', { name: 'Not now' })).toBeVisible();
      // The athlete never reads the name of the service behind the sign-in.
      await expect(sheet.getByText(/sciotte/i)).toHaveCount(0);

      // The choices' ink stands off the sheet in each theme.
      const inks = await sheet.getByTestId('bring-history-sign-in').evaluate((choice) => {
        const style = (el: Element) => getComputedStyle(el).color;
        const label = choice.querySelector('span span');
        return { label: label ? style(label) : '', panel: getComputedStyle(choice.closest('[role="dialog"]') as Element).backgroundColor };
      });
      expect(inks.label).not.toBe(inks.panel);

      await page.waitForTimeout(500);
      await sheet.screenshot({ path: testInfo.outputPath(`bring-history-sheet-${theme}.png`) });
    });
  }

  test('its sign-in opens the Garmin login, saying it copies the history once', async ({ page }) => {
    await mockCards(page, 'garmin_archive');
    const sheet = await openSheet(page);

    await sheet.getByTestId('bring-history-sign-in').click();

    await expect(sheet).toHaveCount(0);
    const login = page.getByRole('dialog', { name: /Garmin/ });
    await expect(login).toBeVisible();
    await expect(login.getByTestId('sciotte-history-intro')).toHaveText(
      'Sign in once to copy your history; Garmin keeps your new workouts coming.',
    );
  });

  test('its export opens the Garmin import in a window of its own', async ({ page, context }) => {
    await mockCards(page, 'garmin_archive');
    const sheet = await openSheet(page);

    const opened = context.waitForEvent('page');
    await sheet.getByTestId('bring-history-archive').click();
    const window = await opened;

    await expect(window).toHaveURL(/\/import#garmin$/);
  });

  test('offers no export when the server names no source, and Not now closes it', async ({ page }) => {
    await mockCards(page, null);
    const sheet = await openSheet(page);

    await expect(sheet.getByTestId('bring-history-archive')).toHaveCount(0);
    await expect(page.getByTestId('provider-export-sciotte_garmin')).toHaveCount(0);
    await sheet.getByRole('button', { name: 'Not now' }).click();

    await expect(sheet).toHaveCount(0);
    await expect(page.getByRole('dialog')).toHaveCount(0);
  });
});
