// ABOUTME: The super-admin fuel-product review drawer against the mocked admin API: every stored photo, the reader's notes and the identity fields
// ABOUTME: Guards what a reviewer needs to judge an OCR read — the photos it came from and what the reader said — and that an identity edit is sent (carnet#155)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { test, expect } from '@playwright/test';
import { setupDashboardMocks, loginToDashboard, navigateToTab } from './test-helpers';

test.describe('fuel-product review drawer', () => {
  test('shows every photo and the reader notes, and sends an identity correction', async ({ page }) => {
    await setupDashboardMocks(page, { role: 'super_admin' });
    let approved: unknown = null;
    await page.route('**/api/admin/fuel-products/proposals/*/approve', async (route) => {
      approved = route.request().postDataJSON();
      await route.fulfill({
        status: 409,
        contentType: 'application/json',
        body: JSON.stringify({ outcome: 'identity_conflict', error: 'conflict', product_id: 'p-sweep' }),
      });
    });
    await loginToDashboard(page);
    await navigateToTab(page, 'Fuel products');

    await page.getByRole('button', { name: 'Review' }).click();
    const drawer = page.getByTestId('fuel-proposal-drawer');
    await expect(drawer.getByTestId('fuel-proposal-image')).toHaveCount(2);
    await expect(drawer.getByTestId('fuel-proposal-photo-1')).toContainText('Photo 2 of 2');
    await expect(drawer.getByTestId('fuel-proposal-notes')).toContainText('nutrition panel');
    await expect(drawer.getByTestId('fuel-proposal-ingredients')).toContainText('maltodextrin');
    await expect(drawer.getByTestId('fuel-proposal-blend-claim')).toContainText('glucose + fructose blend');

    await drawer.getByLabel('Form').selectOption('puree');
    await drawer.getByRole('button', { name: /Save and approve \(1 edit\)/ }).click();
    await expect(drawer.getByTestId('fuel-review-problem')).toContainText('never files two products under one name or one barcode');
    expect(approved).toEqual({ overrides: { form: 'puree' } });
  });
});
