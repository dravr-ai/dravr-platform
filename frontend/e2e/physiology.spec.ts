// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: E2E for Settings › Physiology — the menu opens the athlete's anchors per sport, read in the app's language, and sets one
// ABOUTME: Runs against a mocked /api/me/physiology shaped like the server's (carnet#1516); an edit PUTs one anchor (carnet#1561)

import { test, expect } from '@playwright/test';
import { setupDashboardMocks, loginToDashboard, PHYSIOLOGY_FIXTURE, PHYSIOLOGY_OFFERED } from './test-helpers';

test.describe('Settings › Physiology', () => {
  test('the settings menu opens each sport family with its anchors', async ({ page }) => {
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
    // Each anchor carries its Edit button, and the pane one to add an anchor.
    await expect(page.getByRole('button', { name: /^Edit · FTP · Cycling/ })).toBeVisible();
    await expect(page.getByTestId('physiology-add')).toBeVisible();
    expect(asked).toContain('en');
  });

  test('the athlete changes an anchor, and the pane reads it back as the server stored it', async ({ page }) => {
    await setupDashboardMocks(page, { role: 'user' });
    const sent: unknown[] = [];
    await page.route('**/api/me/physiology/ride/ftp**', async (route, request) => {
      sent.push(request.postDataJSON());
      const saved = structuredClone(PHYSIOLOGY_FIXTURE);
      const ftp = saved.sports[0].anchors[0];
      Object.assign(ftp, { value: 292, confidence: 'medium', value_text: '292 W' });
      await route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify(saved) });
    });
    await loginToDashboard(page);
    await page.goto('/#settings/physiology');

    await page.getByTestId('physiology-ride-ftp-edit').click();
    const editor = page.getByTestId('physiology-editor');
    await expect(editor.getByTestId('physiology-editor-value')).toHaveValue('285');
    await expect(editor.getByTestId('physiology-editor-save')).toBeDisabled();
    await editor.getByTestId('physiology-editor-value').fill('292');
    await editor.getByTestId('physiology-editor-kind').selectOption('measured');
    await editor.getByTestId('physiology-editor-confidence').selectOption('medium');
    await editor.getByTestId('physiology-editor-save').click();

    await expect(page.getByTestId('physiology-ride-ftp-value')).toHaveText('292 W');
    await expect(page.getByTestId('physiology-editor')).toHaveCount(0);
    expect(sent).toEqual([
      {
        value: 292,
        low: null,
        high: null,
        pace_per: null,
        kind: 'measured',
        confidence: 'medium',
        method: null,
        measured_on: null,
      },
    ]);
  });

  test('an athlete with no anchor is told how to add one', async ({ page }) => {
    await setupDashboardMocks(page, { role: 'user' });
    await page.route('**/api/me/physiology**', (route) =>
      route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({ sports: [], offered: PHYSIOLOGY_OFFERED }),
      }),
    );
    await loginToDashboard(page);
    await page.goto('/#settings/physiology');

    await expect(page.getByTestId('physiology-empty')).toContainText('Add one here');
    await expect(page.getByTestId('physiology-add')).toBeVisible();
  });
});
