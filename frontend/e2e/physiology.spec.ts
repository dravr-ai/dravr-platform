// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: E2E for Settings › Physiology — the menu opens the athlete's anchors per sport, read in the app's language
// ABOUTME: Runs against a mocked /api/me/physiology shaped like the server's (carnet#1516); nothing on the pane edits

import { test, expect } from '@playwright/test';
import { setupDashboardMocks, loginToDashboard, PHYSIOLOGY_FIXTURE } from './test-helpers';

test.describe('Settings › Physiology', () => {
  test('the settings menu opens each sport family with its anchors, read-only', async ({ page }) => {
    await setupDashboardMocks(page, { role: 'user' });
    const asked: string[] = [];
    await page.route('**/api/me/physiology**', async (route, request) => {
      asked.push(new URL(request.url()).searchParams.get('locale') ?? '');
      await route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify(PHYSIOLOGY_FIXTURE) });
    });
    await loginToDashboard(page);

    await page.getByRole('button', { name: 'Settings', exact: true }).click();
    await page.getByTestId('settings-menu-physiology').click();

    const pane = page.getByTestId('physiology-settings');
    await expect(pane.getByRole('heading', { name: 'Cycling' })).toBeVisible();
    await expect(pane.getByRole('heading', { name: 'Running' })).toBeVisible();
    await expect(page.getByTestId('physiology-ride-ftp-value')).toHaveText('about 285 W (estimate)');
    await expect(page.getByTestId('physiology-ride-ftp-source')).toHaveText('your own value');
    await expect(page.getByTestId('physiology-ride-map-facts')).toHaveText(
      'As of Nov 2, 2026 · Range 365–395 W · High confidence',
    );
    await expect(page.getByTestId('physiology-ride-max_heart_rate-facts')).toHaveText(
      'Date not recorded · Low confidence',
    );
    await expect(page.getByTestId('physiology-run-critical_speed-value')).toHaveText('about 4:05–4:15/km (estimate)');
    // Read-only: the pane offers no control to change a value.
    await expect(pane.getByRole('button')).toHaveCount(0);
    await expect(pane.getByRole('textbox')).toHaveCount(0);
    expect(asked).toContain('en');
  });

  test('an athlete with no anchor is told how to add one', async ({ page }) => {
    await setupDashboardMocks(page, { role: 'user' });
    await page.route('**/api/me/physiology**', (route) =>
      route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify({ sports: [] }) }),
    );
    await loginToDashboard(page);
    await page.goto('/#settings/physiology');

    await expect(page.getByTestId('physiology-empty')).toContainText('Tell your agent');
  });
});
