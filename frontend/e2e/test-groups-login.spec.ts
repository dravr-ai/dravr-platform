// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { test, expect } from '@playwright/test';
import { setupDashboardMocks, loginToDashboard } from './test-helpers';

test('can login as user and see Groups tab', async ({ page }) => {
  await setupDashboardMocks(page, { role: 'user' });
  await loginToDashboard(page);
  const groupsTab = page.locator('button').filter({ has: page.locator('text=Groups') });
  await expect(groupsTab).toBeVisible({ timeout: 5000 });
});
