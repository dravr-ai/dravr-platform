// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: E2E spec for the first-run onboarding gate — verifies a user with zero providers cannot reach the dashboard
// ABOUTME: Mocks GET /api/me/onboarding-status with needs_provider_connection=true; asserts the connect-provider screen and its stacking under the offline strip

import { test, expect, type Page } from '@playwright/test';
import { setupDashboardMocks, loginToDashboard } from './test-helpers';

/**
 * Spec-local login that doesn't wait for `<main>` — the onboarding screen
 * intentionally has no `<main>` element, so the shared `loginToDashboard`
 * helper would time out. Returns after the sign-in click; assertions in the
 * test wait for the screen-specific anchor.
 */
async function loginExpectingOnboarding(page: Page, email: string) {
  await page.goto('/');
  await page.waitForSelector('form', { timeout: 10_000 });
  await page.locator('input[name="email"]').fill(email);
  await page.locator('input[name="password"]').fill('password123');
  await page.getByRole('button', { name: 'Sign in' }).click();
}

/**
 * Override the default `needs_provider_connection: false` stub with `true` so
 * the App.tsx route guard treats the test user as a fresh first-run account.
 * Must be called AFTER `setupDashboardMocks` — Playwright's `page.route`
 * uses last-registered-first matching, so the later registration wins.
 */
async function stubOnboardingNeeded(page: Page) {
  await page.route('**/api/me/onboarding-status', async (route) => {
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({ needs_provider_connection: true }),
    });
  });
}

test.describe('Onboarding gate: connect a provider before chatting', () => {
  test('fresh user is redirected to the connect-provider screen instead of the dashboard', async ({ page }) => {
    await setupDashboardMocks(page, { role: 'user', email: 'fresh@test.com', displayName: 'Fresh User' });
    await stubOnboardingNeeded(page);
    await loginExpectingOnboarding(page, 'fresh@test.com');

    // Canonical anchor for the onboarding screen. The welcome heading is
    // unique to `OnboardingConnectProvider` — the dashboard sidebar never
    // renders this text — so visibility here is a clean redirect proof.
    await expect(page.getByRole('heading', { name: /welcome, fresh user/i })).toBeVisible();

    // The connect step wears the same chrome as the other six steps: the
    // shared OnboardingShell, with the welcome as its heading rather than a
    // second logo-and-card layout of its own.
    await expect(page.getByTestId('onboarding-shell')).toBeVisible();
    await expect(
      page.getByTestId('onboarding-shell').getByRole('heading', { name: /welcome, fresh user/i }),
    ).toBeVisible();

    // The onboarding wrapper intentionally omits `onSkip` from
    // ProviderConnectionCards so the skip card never renders. Asserting the
    // absence of its aria-label proves the user cannot bypass the gate.
    await expect(page.locator('[aria-label="Skip and start chatting"]')).toHaveCount(0);

    // Dashboard chrome must NOT be present — the user is fully gated until a
    // provider is connected. The "Groups" sidebar button is the canonical
    // dashboard surface that proves the redirect happened.
    await expect(page.locator('button:has-text("Groups")')).toHaveCount(0);
  });

  test('already-onboarded user lands on the dashboard, not the onboarding screen', async ({ page }) => {
    // Default `setupDashboardMocks` stubs `needs_provider_connection: false`;
    // this guards against regression on the happy path (the value the gate
    // checks is correctly read, and `false` lets the user through).
    await setupDashboardMocks(page, { role: 'user', email: 'returning@test.com', displayName: 'Returning User' });
    await loginToDashboard(page, { email: 'returning@test.com' });

    // The onboarding welcome heading is NOT rendered.
    await expect(page.getByRole('heading', { name: /welcome, returning user/i })).toHaveCount(0);
    // Dashboard chrome IS rendered.
    await expect(page.locator('main')).toBeVisible();
  });

  test('offline, the progress bar stacks under the offline strip and the step clears both, with and without a notch', async ({ page, context }) => {
    await setupDashboardMocks(page, { role: 'user', email: 'fresh@test.com', displayName: 'Fresh User' });
    await stubOnboardingNeeded(page);
    await loginExpectingOnboarding(page, 'fresh@test.com');
    const heading = page.getByRole('heading', { name: /welcome, fresh user/i });
    await expect(heading).toBeVisible();

    const progress = page.getByRole('navigation', { name: 'Onboarding progress' });
    await expect(progress).toBeVisible();

    await context.setOffline(true);
    const strip = page.getByTestId('offline-banner');
    await expect(strip).toBeVisible();

    /** Strip, then progress bar, then the step's heading — each below the last. */
    async function expectStacked(notch: number) {
      const stripBox = await strip.boundingBox();
      const barBox = await progress.boundingBox();
      const headingBox = await heading.boundingBox();
      expect(stripBox).not.toBeNull();
      expect(barBox).not.toBeNull();
      expect(headingBox).not.toBeNull();
      // The strip is topmost: it alone clears the notch.
      expect(stripBox?.y).toBe(0);
      expect(await strip.evaluate((el) => getComputedStyle(el).borderTopWidth)).toBe(`${notch}px`);
      // The bar starts where the strip ends, never under it.
      expect(barBox?.y ?? -1).toBeGreaterThanOrEqual((stripBox?.y ?? 0) + (stripBox?.height ?? 0) - 0.5);
      // The step reserves room for both: its heading clears the bar.
      expect(headingBox?.y ?? -1).toBeGreaterThanOrEqual((barBox?.y ?? 0) + (barBox?.height ?? 0));
    }

    await expectStacked(0);

    // An installed iOS PWA on a notched iPhone: a 47px status bar over y=0.
    const cdp = await context.newCDPSession(page);
    await cdp.send('Emulation.setSafeAreaInsetsOverride', { insets: { top: 47 } });
    await expect.poll(() => strip.evaluate((el) => getComputedStyle(el).borderTopWidth)).toBe('47px');
    await expectStacked(47);
    // Under the strip, the bar does not pad for the notch a second time.
    expect(await progress.evaluate((el) => getComputedStyle(el).borderTopWidth)).toBe('0px');

    // Back online the strip goes, and the bar and the step take the inset
    // themselves: both clear the notch, the heading still clears the bar.
    await context.setOffline(false);
    await expect(strip).toHaveCount(0);
    await expect.poll(() => progress.evaluate((el) => el.getBoundingClientRect().top)).toBe(0);
    expect(await progress.evaluate((el) => getComputedStyle(el).borderTopWidth)).toBe('47px');
    const barBox = await progress.boundingBox();
    const headingBox = await heading.boundingBox();
    expect(headingBox?.y ?? -1).toBeGreaterThanOrEqual((barBox?.y ?? 0) + (barBox?.height ?? 0));
  });
});
