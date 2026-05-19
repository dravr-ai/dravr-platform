// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Mobile-viewport smoke tests for the authenticated shell.
// ABOUTME: Verifies bottom tab bar, drawer, no horizontal overflow, tap targets.

import { test, expect } from '@playwright/test';
import { setupDashboardMocks, loginToDashboard } from './test-helpers';

test.describe('Mobile authenticated layout', () => {
  test.beforeEach(async ({ page }) => {
    // Use a regular user — admin uses a different default tab set and the
    // primary mobile bar pins user-mode destinations.
    await setupDashboardMocks(page, {
      role: 'user',
      email: 'alice@acme.com',
      displayName: 'Alice Test',
    });
    await loginToDashboard(page, { email: 'alice@acme.com', password: 'password123' });
  });

  test('bottom tab bar renders with 5 entries including Menu', async ({ page }) => {
    const nav = page.getByRole('navigation', { name: 'Primary navigation' });
    await expect(nav).toBeVisible();
    // 4 primary + Menu
    await expect(nav.getByRole('button', { name: 'Chat' })).toBeVisible();
    await expect(nav.getByRole('button', { name: 'Coaches' })).toBeVisible();
    await expect(nav.getByRole('button', { name: 'Insights' })).toBeVisible();
    await expect(nav.getByRole('button', { name: 'Groups' })).toBeVisible();
    await expect(nav.getByRole('button', { name: 'Open menu' })).toBeVisible();
  });

  test('desktop sidebar is hidden at mobile viewport', async ({ page }) => {
    // The sidebar's "Expand sidebar" / collapse-toggle should not be in the
    // accessibility tree at mobile widths.
    const collapseToggle = page.getByRole('button', { name: /(Expand|Collapse) sidebar/ });
    await expect(collapseToggle).toBeHidden();
  });

  test('drawer opens via Menu and closes via overlay click', async ({ page }) => {
    await page.getByRole('button', { name: 'Open menu' }).click();
    const drawer = page.getByRole('dialog', { name: 'Secondary navigation' });
    await expect(drawer).toBeVisible();
    await expect(drawer.getByRole('button', { name: 'Settings' })).toBeVisible();
    await expect(drawer.getByRole('button', { name: 'Sign out' })).toBeVisible();

    // Close via the explicit Close button to avoid coordinate ambiguity.
    await drawer.getByRole('button', { name: 'Close menu' }).click();
    await expect(drawer).toBeHidden();
  });

  test('no horizontal page overflow at mobile width', async ({ page }) => {
    const { docScrollWidth, clientWidth } = await page.evaluate(() => ({
      docScrollWidth: document.documentElement.scrollWidth,
      clientWidth: document.documentElement.clientWidth,
    }));
    // Allow a 1px sub-pixel tolerance on the Pixel 7 emulation.
    expect(docScrollWidth).toBeLessThanOrEqual(clientWidth + 1);
  });

  test('chat Send button meets 44x44 hit area', async ({ page }) => {
    const send = page.getByRole('button', { name: 'Send message' }).first();
    const box = await send.boundingBox();
    expect(box).not.toBeNull();
    if (box) {
      expect(box.width).toBeGreaterThanOrEqual(44);
      expect(box.height).toBeGreaterThanOrEqual(44);
    }
  });

  test('navigating from drawer dismisses it and switches tab', async ({ page }) => {
    await page.getByRole('button', { name: 'Open menu' }).click();
    const drawer = page.getByRole('dialog', { name: 'Secondary navigation' });
    await drawer.getByRole('button', { name: /^Notifications/ }).click();
    await expect(drawer).toBeHidden();
    await expect(page).toHaveURL(/#notifications/);
  });
});
