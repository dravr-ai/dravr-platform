// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Playwright E2E tests for the mailed invitation link (/accept-invite#token=…).
// ABOUTME: The token leaves the address bar, the password is chosen here, and a spent link leads to sign-up.

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

test.describe('Invitation link', () => {
  test('opens the password form and takes the token out of the address bar', async ({ page }) => {
    await setupBasicMocks(page);
    await page.goto(`/accept-invite#token=${TOKEN}`);

    await expect(page.locator('h1')).toContainText('Create Your Account');
    await expect(page.locator('input[name="password"]')).toBeVisible();
    // The invitation is sent to one address: the form never asks for another.
    await expect(page.locator('input[name="email"]')).toHaveCount(0);
    await expect(page).toHaveURL((url) => url.pathname === '/' && url.hash === '');
  });

  test('redeems the token with the chosen password, then offers sign-in', async ({ page }) => {
    await setupBasicMocks(page);
    let sent: Record<string, unknown> | null = null;
    await page.route('**/api/auth/accept-invitation', async (route) => {
      sent = route.request().postDataJSON() as Record<string, unknown>;
      await route.fulfill({
        status: 201,
        contentType: 'application/json',
        body: JSON.stringify({
          user_id: 'user-invited',
          user_status: 'active',
          display_name: 'Coach Phil',
          message: 'Your account is ready.',
        }),
      });
    });

    await page.goto(`/accept-invite#token=${TOKEN}`);
    await page.locator('input[name="displayName"]').fill('Coach Phil');
    await page.locator('input[name="password"]').fill('SecurePassword123');
    await page.locator('input[name="confirmPassword"]').fill('SecurePassword123');
    await page.getByRole('button', { name: 'Create Account' }).click();

    await expect(page.locator('h1')).toContainText('Account created');
    expect(sent).toEqual({ token: TOKEN, password: 'SecurePassword123', display_name: 'Coach Phil' });

    await page.getByRole('button', { name: 'Continue to sign in' }).click();
    await waitForLoginScreen(page, 10000);
  });

  test('a spent link leads to the create-account form', async ({ page }) => {
    await setupBasicMocks(page);
    await page.route('**/api/auth/accept-invitation', (route) =>
      route.fulfill({
        status: 404,
        contentType: 'application/json',
        body: JSON.stringify({ code: 'resource_not_found', message: 'The requested resource was not found' }),
      }),
    );

    await page.goto(`/accept-invite#token=${TOKEN}`);
    await page.locator('input[name="password"]').fill('SecurePassword123');
    await page.locator('input[name="confirmPassword"]').fill('SecurePassword123');
    await page.getByRole('button', { name: 'Create Account' }).click();

    await expect(page.locator('h1')).toContainText('That link has expired');
    await page.getByRole('button', { name: 'Create an account' }).click();
    await expect(page.locator('input[name="email"]')).toBeVisible();
    await expect(page.locator('h1')).toContainText('Create Your Account');
  });
});
