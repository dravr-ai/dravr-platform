// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: E2E for importing your own data (carnet#1218): the link under the providers opens /import, the Strava import's states
// ABOUTME: Runs against a mocked /api/imports shaped like the server's, holding the athlete's import between requests

import { test, expect, type Page, type Route } from '@playwright/test';
import { setupDashboardMocks, loginToDashboard } from './test-helpers';

const json = (route: Route, body: unknown, status = 200) =>
  route.fulfill({ status, contentType: 'application/json', body: JSON.stringify(body) });

interface Job {
  id: string;
  source: string;
  status: string;
  total_rows: number | null;
  processed_rows: number;
  imported: number;
  skipped: number;
  failed: number;
  error_code: string | null;
  earliest_start: string | null;
  latest_start: string | null;
  created_at: string;
  updated_at: string;
  finished_at: string | null;
  expires_at: string | null;
  history_held: boolean;
}

function completed(): Job {
  return {
    id: '44444444-4444-4444-8444-444444444444',
    source: 'strava_archive',
    status: 'completed',
    total_rows: 4,
    processed_rows: 4,
    imported: 2,
    skipped: 0,
    failed: 2,
    error_code: null,
    earliest_start: '2017-06-12T19:32:22Z',
    latest_start: '2026-09-28T11:02:03Z',
    created_at: '2026-10-28T10:00:00Z',
    updated_at: '2026-10-28T10:01:00Z',
    finished_at: '2026-10-28T10:01:00Z',
    expires_at: null,
    history_held: true,
  };
}

/** The athlete's Strava import as the server holds it, changed by each request. */
async function mockImports(page: Page, initial: Job | null) {
  const state = { job: initial };
  await page.route('**/api/imports', (route) =>
    json(route, { enabled: true, sources: ['strava_archive'], imports: state.job ? [state.job] : [] }),
  );
  await page.route('**/api/imports/strava_archive/current', (route) => json(route, { import: state.job }));
  await page.route('**/api/imports/strava_archive/check', (route, request) => {
    const { url } = JSON.parse(request.postData() ?? '{}') as { url: string };
    return url.includes('export.example.com')
      ? json(route, { accepted: true, refusal: null, size_bytes: 1_200_000_000 })
      : json(route, { accepted: false, refusal: 'link_host_not_allowed', size_bytes: null });
  });
  await page.route('**/api/imports/strava_archive', (route) => {
    state.job = completed();
    return json(route, { ...completed(), status: 'queued' }, 201);
  });
  await page.route('**/api/imports/strava_archive/data', (route) => {
    if (state.job) state.job = { ...state.job, history_held: false };
    return json(route, { deleted: true, originals_deleted: 1 });
  });
  return state;
}

test.describe('Import your own data', () => {
  test.beforeEach(async ({ page }) => {
    await setupDashboardMocks(page, { role: 'user', email: 'alice@acme.com', displayName: 'Alice Test' });
    // The import window is a page load of its own (/import): the session is
    // restored on it the way every reload restores one.
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
  });

  test('the link under the providers opens the import window in a window of its own', async ({ page, context }) => {
    await mockImports(page, null);
    await loginToDashboard(page);
    await page.goto('/#settings/connections');

    const opened = context.waitForEvent('page');
    await page.getByTestId('import-own-data-link').click();
    const window = await opened;

    await expect(window).toHaveURL(/\/import$/);
  });

  test('refuses a link off the export host, then imports from one that is, then deletes the history', async ({ page }) => {
    await mockImports(page, null);
    await loginToDashboard(page);
    await page.goto('/import#strava');

    const input = page.getByTestId('import-link-input');
    await input.fill('https://elsewhere.example.org/x.zip');
    await page.getByTestId('import-check').click();
    await expect(page.getByTestId('import-error')).toHaveText("That link isn't from Strava's export email.");

    await input.fill('https://export.example.com/x.zip');
    await page.getByTestId('import-check').click();
    await expect(page.getByTestId('import-link-found')).toHaveText('Export found · about 1.2 GB');
    await page.getByTestId('import-start').click();

    await expect(page.getByTestId('import-completed')).toContainText('Activities imported: 2');
    await page.getByRole('button', { name: 'Delete imported history' }).click();
    await page.getByRole('button', { name: 'Delete 2 activities' }).click();

    await expect(page.getByTestId('import-notice')).toHaveText('Imported history deleted.');
    await expect(page.getByTestId('import-completed')).toHaveCount(0);
    await expect(page.getByTestId('import-link-input')).toBeVisible();
  });
});
