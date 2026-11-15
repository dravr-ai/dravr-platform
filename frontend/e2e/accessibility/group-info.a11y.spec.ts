// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Group info for each of its readers (carnet#1527) — the owner waiting on the seat, the coach, an athlete — in both themes
// ABOUTME: WCAG 2.1 AA with contrast, one filled button at most and no filled red, and the keyboard kept in the sheet

import { test, expect, type Page } from '@playwright/test';
import AxeBuilder from '@axe-core/playwright';
import { openSeatRoom, setupCoachSeatMocks, type SeatMockOptions } from '../coach-seat-mocks';

const VIEWERS: Array<{ name: string; options: SeatMockOptions; filled: number }> = [
  // The seat's request is the drawer's one filled button.
  { name: 'the owner waiting on the seat', options: {}, filled: 1 },
  // The pilot's one athlete: their plan thread is the one filled button, as on coach Home.
  { name: 'the coach', options: { managesRoster: true, coach: 'viewer' }, filled: 1 },
  // Nothing for the athlete to do here is filled.
  { name: 'an athlete', options: { role: 'member', coach: 'someone-else' }, filled: 0 },
];

async function openGroupInfo(page: Page, options: SeatMockOptions) {
  await setupCoachSeatMocks(page, options);
  await openSeatRoom(page);
  await page.getByTestId('conversation-header-title').click();
  await expect(page.getByTestId('group-info-panel')).toBeVisible();
  // The roster's rows are in before anything is measured.
  await expect(page.getByTestId('group-info-agent').first()).toBeVisible();
}

async function setTheme(page: Page, theme: 'light' | 'dark') {
  await page.evaluate((t) => document.documentElement.classList.toggle('dark', t === 'dark'), theme);
  // Let the token transitions land before measuring contrast.
  await page.waitForTimeout(1200);
}

test.describe('Group info accessibility', () => {
  for (const viewer of VIEWERS) {
    for (const theme of ['light', 'dark'] as const) {
      test(`${viewer.name}, ${theme}: no WCAG 2.1 AA violations`, async ({ page }) => {
        await openGroupInfo(page, viewer.options);
        await setTheme(page, theme);
        const results = await new AxeBuilder({ page })
          .include('[data-testid="conversation-info-panel"]')
          .withTags(['wcag2a', 'wcag2aa', 'wcag21aa'])
          .analyze();
        const report = results.violations
          .map((v) => `${v.id} (${v.impact}): ${v.help}\n${v.nodes.slice(0, 4).map((n) => `    ${n.target.join(' ')}`).join('\n')}`)
          .join('\n\n');
        expect(report).toBe('');
      });
    }

    test(`${viewer.name}: ${viewer.filled} filled button at most, and no filled red outside a dialog`, async ({ page }) => {
      await openGroupInfo(page, viewer.options);
      const panel = page.getByTestId('group-info-panel');
      await expect(panel.locator('.btn-primary')).toHaveCount(viewer.filled);
      await expect(panel.locator('.btn-danger')).toHaveCount(0);
    });
  }

  test('Tab stays in the drawer, and Escape gives focus back to the thread title', async ({ page }) => {
    await setupCoachSeatMocks(page);
    await openSeatRoom(page);
    const title = page.getByTestId('conversation-header-title');
    await title.focus();
    await page.keyboard.press('Enter');
    const sheet = page.getByTestId('conversation-info-panel');
    await expect(sheet).toBeVisible();
    await expect(page.getByTestId('group-info-agent')).toBeVisible();

    for (let step = 0; step < 30; step += 1) {
      await page.keyboard.press('Tab');
      expect(await sheet.evaluate((el) => el.contains(document.activeElement))).toBe(true);
    }

    await page.keyboard.press('Escape');
    await expect(sheet).toHaveCount(0);
    await expect(title).toBeFocused();
  });

  test("a confirm opened from the drawer keeps Escape to itself, and the drawer stays open", async ({ page }) => {
    await setupCoachSeatMocks(page);
    await openSeatRoom(page);
    await page.getByTestId('conversation-header-title').click();
    await page.getByTestId('group-info-delete').click();
    await expect(page.getByText('Delete this group?')).toBeVisible();

    await page.keyboard.press('Escape');
    await expect(page.getByText('Delete this group?')).toHaveCount(0);
    await expect(page.getByTestId('conversation-info-panel')).toBeVisible();
    // Focus is back on the row that opened the confirm.
    await expect(page.getByTestId('group-info-delete')).toBeFocused();
  });
});
