// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Follows the athlete's originals bundle on the privacy pane — asked for, followed while it builds, downloaded from its signed URL
// ABOUTME: Pins the refusals the server names by code, the passphrase floor, and the panel in both themes (carnet#1541)

import { test, expect, type Page, type Route } from '@playwright/test';
import { setupDashboardMocks, loginToDashboard } from './test-helpers';

const BUNDLE_ID = 'b-0001';
const SIGNED_URL = 'https://storage.googleapis.com/dravr-originals/b-0001.zip?X-Goog-Signature=abc';

/** A bundle as the server sends it, `status` and counts set per stage. */
function bundle(status: 'building' | 'ready' | 'failed', sealed = false): Record<string, unknown> {
  return {
    bundle_id: BUNDLE_ID,
    status,
    sealed,
    created_at: '2026-11-16T14:00:00Z',
    expires_at: '2026-11-30T14:00:00Z',
    files: status === 'ready' ? 3 : null,
    missing: status === 'ready' ? 0 : null,
    size_bytes: status === 'ready' ? 4096 : null,
    sha256: status === 'ready' ? 'a'.repeat(64) : null,
  };
}

function json(route: Route, status: number, body: unknown): Promise<void> {
  return route.fulfill({ status, contentType: 'application/json', body: JSON.stringify(body) });
}

/** Signs in and opens Settings → Privacy, where the panel lives. */
async function openPrivacy(page: Page): Promise<void> {
  await loginToDashboard(page);
  await page.getByRole('button', { name: 'Settings', exact: true }).first().click();
  await page.getByTestId('settings-menu-privacy').click();
  await expect(page.getByTestId('originals-bundle-panel')).toBeVisible();
}

test.describe('Originals bundle', () => {
  test('asked for, followed while it builds, then downloaded from its signed URL', async ({ page }) => {
    await setupDashboardMocks(page, { role: 'user' });
    let polls = 0;
    const asked: unknown[] = [];
    // Registered after the dashboard mocks, so these win.
    await page.route('**/api/user/export/originals', (route) => {
      if (route.request().method() === 'POST') {
        asked.push(route.request().postDataJSON());
        return json(route, 202, bundle('building'));
      }
      return json(route, 200, null);
    });
    await page.route(`**/api/user/export/originals/${BUNDLE_ID}`, (route) => {
      polls += 1;
      // The first look still finds it building; the next finds it ready.
      return json(route, 200, bundle(polls === 1 ? 'building' : 'ready'));
    });
    await page.route(`**/api/user/export/originals/${BUNDLE_ID}/download`, (route) =>
      json(route, 200, { url: SIGNED_URL, expires_at: '2026-11-16T14:15:00Z' }),
    );
    // The signed URL is storage's, never the app's: answer it as storage would.
    let fetchedSigned = false;
    await page.route('https://storage.googleapis.com/**', (route) => {
      fetchedSigned = true;
      return route.fulfill({
        status: 200,
        contentType: 'application/zip',
        headers: { 'content-disposition': 'attachment; filename="dravr-originals-2026-11-16.zip"' },
        body: 'PK',
      });
    });

    await openPrivacy(page);
    await page.getByTestId('originals-request').click();

    // An empty passphrase asks for no seal.
    expect(asked).toEqual([{}]);
    await expect(page.getByTestId('originals-state')).toContainText('Preparing your files');
    await expect(page.getByTestId('originals-passphrase')).toBeDisabled();

    // The build is followed until it is ready.
    await expect(page.getByTestId('originals-state')).toContainText('Your files are ready (3 files)', {
      timeout: 15_000,
    });
    expect(polls, 'the panel polled the bundle it started until it was ready').toBeGreaterThan(1);

    await page.getByTestId('originals-download').click();
    await expect.poll(() => fetchedSigned, { message: 'the browser was sent to the signed URL' }).toBe(true);
  });

  test('a sealed bundle sends its passphrase and says it is protected', async ({ page }) => {
    await setupDashboardMocks(page, { role: 'user' });
    const asked: unknown[] = [];
    await page.route('**/api/user/export/originals', (route) => {
      if (route.request().method() === 'POST') {
        asked.push(route.request().postDataJSON());
        return json(route, 200, bundle('ready', true));
      }
      return json(route, 200, null);
    });

    await openPrivacy(page);
    const field = page.getByTestId('originals-passphrase');
    await field.fill('too short');
    await expect(page.getByText('Use at least 12 characters.')).toBeVisible();
    await expect(page.getByTestId('originals-request')).toBeDisabled();

    await field.fill('correct horse battery staple');
    await page.getByTestId('originals-request').click();

    expect(asked).toEqual([{ passphrase: 'correct horse battery staple' }]);
    await expect(page.getByTestId('originals-state')).toContainText('protected with your passphrase');
    await expect(field, 'the passphrase is not kept in the field').toHaveValue('');
  });

  for (const [status, code, sentence] of [
    [429, 'export_rate_limited', 'several times today'],
    [503, 'export_busy', 'Too many files are being prepared right now'],
    [409, 'bundle_building', 'Another file is being prepared'],
  ] as const) {
    test(`a ${status} ${code} refusal is named to the athlete`, async ({ page }) => {
      await setupDashboardMocks(page, { role: 'user' });
      await page.route('**/api/user/export/originals', (route) =>
        route.request().method() === 'POST'
          ? json(route, status, { error: code, message: 'refused' })
          : json(route, 200, null),
      );

      await openPrivacy(page);
      await page.getByTestId('originals-request').click();

      await expect(page.getByTestId('originals-failure')).toContainText(sentence);
    });
  }

  for (const theme of ['light', 'dark'] as const) {
    test(`the ready panel reads in the ${theme} theme`, async ({ page }) => {
      await page.addInitScript((value) => window.localStorage.setItem('dravr.theme', value), theme);
      await setupDashboardMocks(page, { role: 'user' });
      await page.route('**/api/user/export/originals', (route) => json(route, 200, bundle('ready')));

      await openPrivacy(page);
      const panel = page.getByTestId('originals-bundle-panel');
      await expect(panel.getByTestId('originals-download')).toBeVisible();
      const html = page.locator('html');
      await (theme === 'dark' ? expect(html).toHaveClass(/\bdark\b/) : expect(html).not.toHaveClass(/\bdark\b/));
      await panel.screenshot({ path: `test-results/originals-bundle-${theme}.png` });
    });
  }
});
