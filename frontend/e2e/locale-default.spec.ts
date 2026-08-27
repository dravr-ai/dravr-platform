// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Proves the product's default locale renders in French, not just that English does
// ABOUTME: The rest of the suite pins English, so without this the default path is untested

import { test, expect } from '@playwright/test';
import { setupDashboardMocks, loginToDashboard } from './test-helpers';

// `DEFAULT_LANGUAGE` and `fallbackLng` are both `fr`: French is what a viewer
// gets before they choose anything, and the majority of the user base reads it.
// Every other spec pins `pierre_app_language=en` so its English assertions hold,
// which would leave the language real users see covered by nothing at all.
//
// Seeding the key here rather than clearing it is deliberate — `applyTestStubs`
// only writes the pin when the key is unset, so an earlier init script wins and
// this file exercises the French path through the same restore the app uses.
test.describe('default locale', () => {
  test.beforeEach(async ({ page }) => {
    await page.addInitScript(() => {
      try {
        window.localStorage.setItem('pierre_app_language', 'fr');
      } catch { /* */ }
    });
    await setupDashboardMocks(page, { role: 'user' });
    await loginToDashboard(page);
  });

  test('renders the chrome in French', async ({ page }) => {
    await page.evaluate(() => { window.location.hash = '#settings'; });
    // Translated copy, asserted by what a French athlete actually reads.
    await expect(page.getByRole('heading', { name: 'Apparence' })).toBeVisible({ timeout: 10000 });
    await expect(page.getByText('Thème', { exact: true })).toBeVisible();
    await expect(page.getByText('Langue', { exact: true })).toBeVisible();
  });

  test('the settings tab strip fits its longer French labels', async ({ page }) => {
    // Translation makes labels longer, and the strip used to scroll rather
    // than wrap at every width: in French "Compte" fell off the right edge at
    // desktop size. The page gutter gate cannot see this — the strip scrolls
    // inside its own box, and that gate measures the document — so the fit is
    // asserted here, in the locale that broke it.
    await page.evaluate(() => { window.location.hash = '#settings'; });
    await expect(page.getByRole('heading', { name: 'Apparence' })).toBeVisible({ timeout: 10000 });

    const clipped = await page.evaluate(() => {
      const nav = document.querySelector('nav[aria-label]');
      if (!nav) {
        return ['no tab strip'];
      }
      const box = nav.getBoundingClientRect();
      return Array.from(nav.querySelectorAll('button'))
        .filter((b) => {
          const r = b.getBoundingClientRect();
          return r.right > box.right + 1 || r.left < box.left - 1;
        })
        .map((b) => (b.textContent ?? '').trim());
    });
    expect(clipped).toEqual([]);
  });

  test('a stored preference is restored outside the screen that owns the picker', async ({ page }) => {
    // The restore effect lives in the switcher hook and the switcher renders
    // only inside Settings, so before it was mounted at the app root a chosen
    // language applied on Settings and nowhere else. Assert the language i18next
    // actually settled on while sitting on a different surface entirely.
    await expect(page.locator('[data-page-shell]')).toBeVisible();
    await expect
      .poll(() => page.evaluate(() => document.documentElement.lang), { timeout: 10000 })
      .toBe('fr');
  });
});
