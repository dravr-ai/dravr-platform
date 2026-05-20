// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: E2E spec for the first-run onboarding gate — verifies a user with zero providers cannot reach the dashboard
// ABOUTME: Mocks GET /api/me/onboarding-status with needs_provider_connection=true and asserts the connect-provider screen

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

    // The onboarding wrapper intentionally omits `onSkip` from
    // ProviderConnectionCards so the skip card never renders. Asserting the
    // absence of its aria-label proves the user cannot bypass the gate.
    await expect(page.locator('[aria-label="Skip and start chatting"]')).toHaveCount(0);

    // Dashboard chrome must NOT be present — the user is fully gated until a
    // provider is connected. The "Chat" sidebar button is the canonical
    // dashboard surface that proves the redirect happened.
    await expect(page.locator('button:has-text("Chat")')).toHaveCount(0);
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
});
