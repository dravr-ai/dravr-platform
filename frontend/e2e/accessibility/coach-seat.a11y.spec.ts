// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: WCAG 2.1 AA coverage for the coach seat (carnet#1520) — the room strip and Group info's block, colour contrast on
// ABOUTME: Each is scanned to ask and while pending, in both themes, so the warning ink and the timeline are measured too

import { test, expect, type Page } from '@playwright/test';
import AxeBuilder from '@axe-core/playwright';
import { PENDING_REQUEST, openSeatRoom, setupCoachSeatMocks } from '../coach-seat-mocks';

async function scan(page: Page, selector: string): Promise<string> {
  const results = await new AxeBuilder({ page })
    .include(selector)
    .withTags(['wcag2a', 'wcag2aa', 'wcag21aa'])
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

test.describe('Coach seat accessibility', () => {
  for (const theme of ['light', 'dark'] as const) {
    for (const [state, request] of [
      ['to ask', null],
      ['pending', PENDING_REQUEST],
    ] as const) {
      test(`the room strip, ${state}, has no WCAG 2.1 AA violations (${theme})`, async ({ page }) => {
        await setupCoachSeatMocks(page, { request });
        await openSeatRoom(page);
        await expect(page.getByTestId('coach-seat-strip')).toBeVisible();
        await setTheme(page, theme);
        expect(await scan(page, '[data-testid="coach-seat-strip"]')).toBe('');
        expect(await scan(page, '[data-testid="thread-header"]')).toBe('');
      });

      test(`Group info's seat block, ${state}, has no WCAG 2.1 AA violations (${theme})`, async ({ page }) => {
        await setupCoachSeatMocks(page, { request });
        await openSeatRoom(page);
        await page.getByTestId('conversation-header-title').click();
        await expect(page.getByTestId('coach-seat-block')).toBeVisible();
        await setTheme(page, theme);
        expect(await scan(page, '[data-testid="coach-seat-block"]')).toBe('');
      });
    }
  }
});
