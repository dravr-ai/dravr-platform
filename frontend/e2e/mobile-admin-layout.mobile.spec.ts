// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Mobile-viewport smoke tests for the admin shell.
// ABOUTME: Admin uses the same drawer/bottom-bar chrome with admin-specific tab anchors.

import { test, expect } from '@playwright/test';
import { setupAndLoginAsAdmin } from './test-helpers';

test.describe('Mobile admin layout', () => {
  test.beforeEach(async ({ page }) => {
    await setupAndLoginAsAdmin(page);
  });

  test('admin bottom tab bar pins Users / Coaches / Coach Store', async ({ page }) => {
    const nav = page.getByRole('navigation', { name: 'Primary navigation' });
    await expect(nav).toBeVisible();
    await expect(nav.getByRole('button', { name: 'Users' })).toBeVisible();
    await expect(nav.getByRole('button', { name: 'Coaches' })).toBeVisible();
    await expect(nav.getByRole('button', { name: 'Coach Store' })).toBeVisible();
    await expect(nav.getByRole('button', { name: 'Open menu' })).toBeVisible();
    await expect(nav.getByRole('button', { name: 'Groups' })).toHaveCount(0);
  });

  test('admin drawer exposes Configuration tabs and Settings', async ({ page }) => {
    await page.getByRole('button', { name: 'Open menu' }).click();
    const drawer = page.getByRole('dialog', { name: 'Secondary navigation' });
    await expect(drawer).toBeVisible();
    await expect(drawer.getByRole('button', { name: /^Tool Management/ })).toBeVisible();
    await expect(drawer.getByRole('button', { name: /^Prompts/ })).toBeVisible();
    await expect(drawer.getByRole('button', { name: /^Settings/ })).toBeVisible();
  });

  test('no horizontal overflow in admin Users tab', async ({ page }) => {
    // Default admin landing is Users — assert layout fits.
    await page.waitForTimeout(400);
    const { docScrollWidth, clientWidth } = await page.evaluate(() => ({
      docScrollWidth: document.documentElement.scrollWidth,
      clientWidth: document.documentElement.clientWidth,
    }));
    expect(docScrollWidth).toBeLessThanOrEqual(clientWidth + 1);
  });
});
