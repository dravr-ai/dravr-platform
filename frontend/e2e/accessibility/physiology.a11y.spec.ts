// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: WCAG 2.1 AA coverage for Settings › Physiology with colour contrast enabled, in both themes
// ABOUTME: Scans a low-confidence estimate, a measured value with its range and a pace range, and the empty state

import { test, expect, type Page } from '@playwright/test';
import AxeBuilder from '@axe-core/playwright';
import { setupDashboardMocks, loginToDashboard } from '../test-helpers';

/** Colour contrast stays ON; the scan covers the pane itself, not the shell around it. */
async function scan(page: Page, testId: string) {
  const results = await new AxeBuilder({ page })
    .withTags(['wcag2a', 'wcag2aa', 'wcag21aa'])
    .include(`[data-testid="${testId}"]`)
    .analyze();
  return results.violations
    .map((v) => `${v.id} (${v.impact}): ${v.help}\n${v.nodes.slice(0, 4).map((n) => `    ${n.target.join(' ')}`).join('\n')}`)
    .join('\n\n');
}

async function setTheme(page: Page, theme: 'light' | 'dark') {
  await page.evaluate((t) => document.documentElement.classList.toggle('dark', t === 'dark'), theme);
  // Let the token transitions land before measuring contrast.
  await page.waitForTimeout(1200);
}

test.describe('Physiology accessibility', () => {
  for (const theme of ['light', 'dark'] as const) {
    test(`Settings › Physiology has no WCAG 2.1 AA violations (${theme})`, async ({ page }) => {
      // The shared mocks answer with a ride and a run family (PHYSIOLOGY_FIXTURE).
      await setupDashboardMocks(page, { role: 'user', email: 'alice@acme.com', displayName: 'Alice Test' });
      await loginToDashboard(page);
      await page.goto('/#settings/physiology');
      await expect(page.getByTestId('physiology-ride-map-value')).toHaveText('380 W');
      await expect(page.getByTestId('physiology-run-critical_speed')).toBeVisible();
      await setTheme(page, theme);

      expect(await scan(page, 'physiology-settings')).toBe('');
    });

    test(`the empty Physiology pane has no WCAG 2.1 AA violations (${theme})`, async ({ page }) => {
      await setupDashboardMocks(page, { role: 'user', email: 'alice@acme.com', displayName: 'Alice Test' });
      await page.route('**/api/me/physiology**', (route) =>
        route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify({ sports: [] }) }),
      );
      await loginToDashboard(page);
      await page.goto('/#settings/physiology');
      await expect(page.getByTestId('physiology-empty')).toBeVisible();
      await setTheme(page, theme);

      expect(await scan(page, 'physiology-settings')).toBe('');
    });
  }
});
