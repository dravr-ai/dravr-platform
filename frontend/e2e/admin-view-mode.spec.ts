// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: E2E for the choice an admin makes at every web sign-in (carnet#1631): the athlete app on their own account, or the console
// ABOUTME: The choice survives a reload, the menu switches it, sign-out forgets it, and an athlete is never asked

import { test, expect, type Page } from '@playwright/test';
import {
  APP_SHELL_TIMEOUT_MS,
  pickViewModeAtSignIn,
  setupDashboardMocks,
  signInThroughHostedPage,
} from './test-helpers';

const ADMIN_SESSION_USER = {
  id: 'user-123',
  user_id: 'user-123',
  email: 'admin@test.com',
  display_name: 'Test Admin',
  role: 'admin',
  is_admin: true,
  user_status: 'active',
  tier: 'professional',
  tenant_id: 'user-123',
};

/** Records every console call, answered as the server answers them for an admin. */
async function recordAdminCalls(page: Page): Promise<string[]> {
  const calls: string[] = [];
  await page.route('**/api/admin/**', async (route) => {
    calls.push(new URL(route.request().url()).pathname);
    await route.fallback();
  });
  return calls;
}

/** A reload restores the session from the cookie: the same admin. */
async function mockSessionRestore(page: Page) {
  await page.route('**/api/auth/session', (route) =>
    route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({ access_token: 'test-jwt-token', csrf_token: 'test-csrf-token', user: ADMIN_SESSION_USER }),
    }),
  );
}

async function signInAsAdminAndWaitForPicker(page: Page) {
  await setupDashboardMocks(page, { role: 'admin', viewMode: 'ask' });
  await page.goto('/');
  await signInThroughHostedPage(page);
  await expect(page.getByTestId('view-mode-picker')).toBeVisible({ timeout: APP_SHELL_TIMEOUT_MS });
}

test.describe('An admin chooses the app at sign-in', () => {
  test('the athlete mode opens Home on the athlete shell, with no console calls', async ({ page }) => {
    const adminCalls = await recordAdminCalls(page);
    await signInAsAdminAndWaitForPicker(page);
    // Nothing of the console is read while the admin is choosing.
    expect(adminCalls).toEqual([]);

    await page.getByTestId('view-mode-athlete').click();

    await expect(page.getByTestId('home-page')).toBeVisible({ timeout: APP_SHELL_TIMEOUT_MS });
    await expect(page).toHaveURL(/#home(\/|$)/);
    await expect(page.getByTestId('icon-rail')).toBeVisible();
    await expect(page.locator('nav button').filter({ has: page.locator('span:text-is("Users")') })).toHaveCount(0);
    expect(adminCalls, `athlete mode issued ${adminCalls.length} console calls`).toEqual([]);
  });

  test('the admin mode opens the console on Users', async ({ page }) => {
    await signInAsAdminAndWaitForPicker(page);

    await page.getByTestId('view-mode-admin').click();

    await expect(page.locator('h1')).toContainText('Users', { timeout: APP_SHELL_TIMEOUT_MS });
    await expect(page.getByTestId('icon-rail')).toHaveCount(0);
  });

  test('a console tab typed in the athlete mode lands on Home', async ({ page }) => {
    const adminCalls = await recordAdminCalls(page);
    await signInAsAdminAndWaitForPicker(page);
    await page.getByTestId('view-mode-athlete').click();
    await expect(page.getByTestId('home-page')).toBeVisible({ timeout: APP_SHELL_TIMEOUT_MS });

    await page.evaluate(() => {
      window.location.hash = '#users';
    });

    await expect(page.getByTestId('home-page')).toBeVisible();
    await expect(page.getByRole('button', { name: 'All Users' })).toHaveCount(0);
    expect(adminCalls).toEqual([]);
  });

  test('the choice survives a reload', async ({ page }) => {
    await signInAsAdminAndWaitForPicker(page);
    await page.getByTestId('view-mode-athlete').click();
    await expect(page.getByTestId('home-page')).toBeVisible({ timeout: APP_SHELL_TIMEOUT_MS });

    await mockSessionRestore(page);
    await page.reload();

    await expect(page.getByTestId('home-page')).toBeVisible({ timeout: APP_SHELL_TIMEOUT_MS });
    await expect(page.getByTestId('view-mode-picker')).toHaveCount(0);
  });

  test('the account menu switches the athlete app to the console, and back', async ({ page }) => {
    await signInAsAdminAndWaitForPicker(page);
    await page.getByTestId('view-mode-athlete').click();
    await expect(page.getByTestId('home-page')).toBeVisible({ timeout: APP_SHELL_TIMEOUT_MS });

    await page.getByTestId('rail-account-menu').click();
    await page.getByTestId('switch-view-mode').click();

    await expect(page.locator('h1')).toContainText('Users', { timeout: APP_SHELL_TIMEOUT_MS });
    await expect(page.getByTestId('icon-rail')).toHaveCount(0);

    await page.getByTestId('switch-view-mode').click();

    await expect(page.getByTestId('home-page')).toBeVisible({ timeout: APP_SHELL_TIMEOUT_MS });
    await expect(page).toHaveURL(/#home(\/|$)/);
  });

  test('signing out forgets the choice: the next sign-in asks again', async ({ page }) => {
    await signInAsAdminAndWaitForPicker(page);
    await page.getByTestId('view-mode-admin').click();
    await expect(page.locator('h1')).toContainText('Users', { timeout: APP_SHELL_TIMEOUT_MS });

    await page.getByRole('button', { name: 'Sign out' }).first().click();
    pickViewModeAtSignIn(page, 'ask');
    await signInThroughHostedPage(page);

    await expect(page.getByTestId('view-mode-picker')).toBeVisible({ timeout: APP_SHELL_TIMEOUT_MS });
  });

  test('an athlete is never asked', async ({ page }) => {
    await setupDashboardMocks(page, { role: 'user', email: 'alice@acme.com', displayName: 'Alice' });
    await page.goto('/');
    await signInThroughHostedPage(page);

    await expect(page.getByTestId('home-page')).toBeVisible({ timeout: APP_SHELL_TIMEOUT_MS });
    await expect(page.getByTestId('view-mode-picker')).toHaveCount(0);
    await page.getByTestId('rail-account-menu').click();
    await expect(page.getByTestId('switch-view-mode')).toHaveCount(0);
  });
});
