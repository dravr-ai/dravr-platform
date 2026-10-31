// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: WCAG 2.1 AA coverage for a coach's Home — À traiter with an athlete open, the team table, the season, and the phone's list — with colour contrast on
// ABOUTME: Each view is scanned in both themes against the mocked roster, tall enough that every row shape reaches the scan

import { test, expect, type Page } from '@playwright/test';
import AxeBuilder from '@axe-core/playwright';
import { mockCoach } from '../coach-home-mocks';

async function scan(page: Page): Promise<string> {
  const results = await new AxeBuilder({ page }).withTags(['wcag2a', 'wcag2aa', 'wcag21aa']).analyze();
  return results.violations
    .map((v) => `${v.id} (${v.impact}): ${v.help}\n${v.nodes.slice(0, 4).map((n) => `    ${n.target.join(' ')}`).join('\n')}`)
    .join('\n\n');
}

async function setTheme(page: Page, theme: 'light' | 'dark') {
  await page.evaluate((t) => document.documentElement.classList.toggle('dark', t === 'dark'), theme);
  // Let the token transitions land before measuring contrast.
  await page.waitForTimeout(1200);
}

test.describe('Coach Home accessibility', () => {
  for (const theme of ['light', 'dark'] as const) {
    test(`À traiter with an athlete open has no WCAG 2.1 AA violations (${theme})`, async ({ page }) => {
      await mockCoach(page, { fillers: 6 });
      await page.getByTestId('coach-queue-row-u-lea').click();
      await expect(page.getByTestId('form-strip')).toBeVisible();
      await setTheme(page, theme);
      expect(await scan(page)).toBe('');
    });

    test(`À traiter on a treated athlete has no WCAG 2.1 AA violations (${theme})`, async ({ page }) => {
      await mockCoach(page);
      await page.getByTestId('coach-queue-row-u-julien').click();
      await page.getByTestId('coach-treat').click();
      await expect(page.getByTestId('coach-treated-note')).toBeVisible();
      await setTheme(page, theme);
      expect(await scan(page)).toBe('');
    });

    test(`the team table has no WCAG 2.1 AA violations (${theme})`, async ({ page }) => {
      await mockCoach(page, { fillers: 6 });
      await page.getByTestId('coach-see-all').click();
      await expect(page.getByTestId('coach-roster')).toBeVisible();
      await setTheme(page, theme);
      expect(await scan(page)).toBe('');
    });

    test(`the season has no WCAG 2.1 AA violations (${theme})`, async ({ page }) => {
      await mockCoach(page, { fillers: 6 });
      await page.getByTestId('coach-view-season').click();
      await expect(page.getByTestId('coach-season-ahead')).toBeVisible();
      await setTheme(page, theme);
      expect(await scan(page)).toBe('');
    });

    test(`the phone's queue and team list have no WCAG 2.1 AA violations (${theme})`, async ({ page }) => {
      await page.setViewportSize({ width: 390, height: 844 });
      await mockCoach(page);
      await expect(page.getByTestId('coach-queue')).toBeVisible();
      await setTheme(page, theme);
      expect(await scan(page)).toBe('');
      await page.getByTestId('coach-see-all').click();
      await expect(page.getByTestId('coach-roster-list')).toBeVisible();
      expect(await scan(page)).toBe('');
    });
  }
});
