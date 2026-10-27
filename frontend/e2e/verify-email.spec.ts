// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Playwright E2E tests for the mailed address-confirmation link (/verify-email#token=…).
// ABOUTME: Opening the link spends nothing, the button confirms, and an expired link re-sends from its own token.

import { test, expect } from '@playwright/test';
import { waitForLoginScreen } from './test-helpers';

const TOKEN = 'AbCd1234EfGh5678.IjKl9012MnOp3456QrSt7890UvWx1234';

async function setupBasicMocks(page: import('@playwright/test').Page) {
  await page.route('**/admin/setup/status', async (route) => {
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({ needs_setup: false, admin_user_exists: true, message: 'Admin user configured' }),
    });
  });
  await page.route('**/api/auth/session', (route) =>
    route.fulfill({ status: 401, contentType: 'application/json', body: '{}' }),
  );
}

test.describe('Address-confirmation link', () => {
  test('opens the page without confirming, and takes the token out of the address bar', async ({ page }) => {
    await setupBasicMocks(page);
    let confirmCalls = 0;
    await page.route('**/api/auth/verify-email', (route) => {
      confirmCalls += 1;
      return route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify({ status: 'verified' }) });
    });

    await page.goto(`/verify-email#token=${TOKEN}`);

    await expect(page.locator('h1')).toContainText('Confirm your email');
    await expect(page.getByRole('button', { name: 'Confirm my address' })).toBeVisible();
    await expect(page.locator('input[type="email"]')).toHaveCount(0);
    await expect(page).toHaveURL((url) => url.pathname === '/' && url.hash === '');
    expect(confirmCalls, 'loading the page must not spend the single-use token').toBe(0);
  });

  test('confirms on the press, then offers sign-in', async ({ page }) => {
    await setupBasicMocks(page);
    let sent: Record<string, unknown> | null = null;
    await page.route('**/api/auth/verify-email', async (route) => {
      sent = route.request().postDataJSON() as Record<string, unknown>;
      await route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify({ status: 'verified' }) });
    });

    await page.goto(`/verify-email#token=${TOKEN}`);
    await page.getByRole('button', { name: 'Confirm my address' }).click();

    await expect(page.locator('h1')).toContainText('Email confirmed');
    expect(sent).toEqual({ token: TOKEN });

    await page.getByRole('button', { name: 'Continue to sign in' }).click();
    await waitForLoginScreen(page, 10000);
  });

  test('an expired link re-sends from its own token without asking for the address', async ({ page }) => {
    await setupBasicMocks(page);
    await page.route('**/api/auth/verify-email', (route) =>
      route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify({ status: 'expired' }) }),
    );
    let resent: Record<string, unknown> | null = null;
    await page.route('**/api/auth/resend-verification', async (route) => {
      resent = route.request().postDataJSON() as Record<string, unknown>;
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({ message: "If that address needs confirming, we've sent a new link." }),
      });
    });

    await page.goto(`/verify-email#token=${TOKEN}`);
    await page.getByRole('button', { name: 'Confirm my address' }).click();

    await expect(page.locator('h1')).toContainText('That link has expired');
    await expect(page.locator('input[type="email"]')).toHaveCount(0);
    await page.getByRole('button', { name: 'Send a new link' }).click();

    await expect(page.getByRole('status')).toBeVisible();
    expect(resent).toEqual({ token: TOKEN });
  });
});
