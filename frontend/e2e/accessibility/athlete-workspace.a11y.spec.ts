// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: WCAG 2.1 AA coverage for a coach's athlete workspace — the calendar with Dravr's proposals, the pre-publish check, each panel tab, the phone's day list (carnet#1816)
// ABOUTME: Scanned in both themes, colour contrast on, against Léa's mocked draft and room

import { test, expect, type Page } from '@playwright/test';
import AxeBuilder from '@axe-core/playwright';
import { mockCoach } from '../coach-home-mocks';
import { WORKSPACE_ATHLETE_ID, mockWorkspace, trifectaRoster } from '../athlete-workspace-mocks';

async function scan(page: Page): Promise<string> {
  // A sheet slides in and its scrim fades in: a scan taken mid-way measures
  // half-transparent text over a half-drawn ground. Scan the page at rest;
  // looping animations (the spinner) never finish, so only finite ones are
  // waited on.
  await page.waitForFunction(() =>
    document
      .getAnimations()
      .filter((animation) => animation.effect?.getComputedTiming().iterations !== Infinity)
      .every((animation) => animation.playState !== 'running'),
  );
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

/** Open Léa's workspace from her row in À traiter, as a coach does. */
async function openWorkspace(page: Page) {
  await mockCoach(page, { roster: trifectaRoster, routes: (routed) => mockWorkspace(routed) });
  await page.getByTestId(`coach-queue-row-${WORKSPACE_ATHLETE_ID}`).click();
  await page.getByTestId('coach-open-workspace').click();
  await expect(page.getByTestId('workspace-calendar')).toBeVisible();
  await expect(page.getByTestId('workspace-prepublish')).toBeVisible();
}

test.describe('athlete workspace accessibility', () => {
  for (const theme of ['light', 'dark'] as const) {
    test(`the calendar and each panel tab have no WCAG 2.1 AA violations (${theme})`, async ({ page }) => {
      await page.setViewportSize({ width: 1440, height: 960 });
      await openWorkspace(page);
      await expect(page.getByTestId('workspace-history-event').first()).toBeVisible();
      await setTheme(page, theme);
      expect(await scan(page)).toBe('');

      await page.locator('#athlete-workspace-tab-dravr').click();
      await expect(page.getByTestId('plan-change-entry')).toBeVisible();
      expect(await scan(page)).toBe('');

      await page.locator('#athlete-workspace-tab-group').click();
      await expect(page.getByTestId('workspace-panel-group').getByTestId('embedded-chat')).toBeVisible();
      expect(await scan(page)).toBe('');
    });

    test(`the phone's day list and panel sheet have no WCAG 2.1 AA violations (${theme})`, async ({ page }) => {
      await page.setViewportSize({ width: 390, height: 844 });
      await openWorkspace(page);
      await setTheme(page, theme);
      expect(await scan(page)).toBe('');

      await page.getByTestId('workspace-open-history').click();
      await expect(page.getByTestId('workspace-panel-sheet')).toBeVisible();
      expect(await scan(page)).toBe('');
    });
  }
});
