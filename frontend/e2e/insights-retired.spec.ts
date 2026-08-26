// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Locks the retirement of the Insights and Friends surfaces (Chat-First Cutover, 2026-08-26)
// ABOUTME: Asserts the nav has no Insights entry, a stale #insights hash lands on chat, and /api/social is never called

import { test, expect } from '@playwright/test';
import { setupDashboardMocks, loginToDashboard } from './test-helpers';

/**
 * Insights (the coach-mediated social feed) and Friends were deleted outright:
 * backend routes, the api-client domain, and both clients. These tests replace
 * the friends / manual-insights suites that drove the old surface, and every
 * assertion here is unconditional — a retired affordance that quietly came
 * back would fail loudly rather than pass behind an `if visible` guard.
 */
test.describe('Insights and Friends are retired', () => {
  test.beforeEach(async ({ page }) => {
    await page.route((url) => url.pathname.startsWith('/api/') || url.pathname.startsWith('/oauth/'), async (route) => {
      await route.fallback();
    });
    await setupDashboardMocks(page, { role: 'user' });
    await page.route('**/api/providers', async (route) => {
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({ providers: [] }),
      });
    });
    await page.route('**/api/coaches**', async (route) => {
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({ coaches: [], total: 0 }),
      });
    });
  });

  test('the sidebar offers no Insights or Friends destination', async ({ page }) => {
    await loginToDashboard(page);
    await page.waitForSelector('aside', { timeout: 10000 });

    const aside = page.locator('aside');
    await expect(aside.getByRole('button', { name: 'Chat' })).toBeVisible();
    await expect(aside.getByRole('button', { name: 'Groups' })).toBeVisible();
    await expect(aside.getByRole('button', { name: 'Insights' })).toHaveCount(0);
    await expect(aside.getByRole('button', { name: 'Friends' })).toHaveCount(0);
    await expect(page.getByRole('button', { name: 'Find Friends' })).toHaveCount(0);
  });

  test('a stale #insights deep link lands on chat', async ({ page }) => {
    await loginToDashboard(page);
    await page.waitForSelector('aside', { timeout: 10000 });

    await page.goto('/#insights/friends');
    await page.waitForSelector('aside', { timeout: 10000 });

    // Nothing serves the tab any more, so the shell renders the chat surface
    // and the retired names appear nowhere on the page.
    await expect(page.getByPlaceholder('Message Dravr...').first()).toBeVisible({ timeout: 10000 });
    await expect(page.getByRole('heading', { name: /friends/i })).toHaveCount(0);
    await expect(page.getByText('No Insights Yet')).toHaveCount(0);
  });

  test('loading the dashboard never calls the deleted social API', async ({ page }) => {
    const socialCalls: string[] = [];
    page.on('request', (request) => {
      if (request.url().includes('/api/social/')) {
        socialCalls.push(request.url());
      }
    });

    await loginToDashboard(page);
    await page.waitForSelector('aside', { timeout: 10000 });
    await page.goto('/#insights');
    await page.waitForSelector('aside', { timeout: 10000 });
    await page.waitForLoadState('networkidle');

    expect(socialCalls).toEqual([]);
  });
});
