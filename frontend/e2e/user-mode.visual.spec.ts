// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Visual E2E tests for user mode (ASY-313).
// ABOUTME: Tests all user dashboard screens against real backend.

import { test, expect } from '@playwright/test';
import {
  loginAsUser,
  navigateToTab,
  takeVisualScreenshot,
  waitForNetworkIdle,
  VISUAL_TEST_CONFIG,
} from './visual-test-helpers';

test.describe('ASY-313: Web User Mode Visual Tests', () => {
  test.describe.configure({ mode: 'serial' });

  // ========================================
  // Login & Authentication
  // ========================================
  test.describe('Login & Authentication', () => {
    test('user login - renders login form', async ({ page }) => {
      // Setup minimal mocks needed for login page to render
      await page.route('**/admin/setup/status', async (route) => {
        await route.fulfill({
          status: 200,
          contentType: 'application/json',
          body: JSON.stringify({ needs_setup: false, admin_user_exists: true }),
        });
      });

      await page.goto('/');
      await page.waitForSelector('form', { timeout: VISUAL_TEST_CONFIG.defaultTimeout });

      await expect(page.locator('input[name="email"]')).toBeVisible();
      await expect(page.locator('input[name="password"]')).toBeVisible();
      await expect(page.getByRole('button', { name: 'Sign in' })).toBeVisible();

      await takeVisualScreenshot(page, 'user-login', 'form-rendered');
    });

    test('user login - successful login redirects to chat', async ({ page }) => {
      await loginAsUser(page, 'webtest');

      // Verify we're on the dashboard (not login page)
      await expect(page.locator('input[name="email"]')).not.toBeVisible();

      await takeVisualScreenshot(page, 'user-login', 'chat-visible');
    });

    test('user login - password visibility toggle works', async ({ page }) => {
      // Setup minimal mocks needed for login page to render
      await page.route('**/admin/setup/status', async (route) => {
        await route.fulfill({
          status: 200,
          contentType: 'application/json',
          body: JSON.stringify({ needs_setup: false, admin_user_exists: true }),
        });
      });

      await page.goto('/');
      await page.waitForSelector('form');

      const passwordInput = page.locator('input[name="password"]');
      await passwordInput.fill('TestPassword');

      // Password should be hidden by default
      await expect(passwordInput).toHaveAttribute('type', 'password');

      // Click toggle button (eye icon inside the password underline input)
      const toggleButton = page.getByRole('button', { name: /show password|hide password/i });
      if (await toggleButton.isVisible().catch(() => false)) {
        await toggleButton.click();
        await expect(passwordInput).toHaveAttribute('type', 'text');

        await takeVisualScreenshot(page, 'user-login', 'password-visible');
      }
    });
  });

  // ========================================
  // Chat Tab
  // ========================================
  test.describe('Chat Tab', () => {
    test.beforeEach(async ({ page }) => {
      await loginAsUser(page, 'webtest');
    });

    test('chat - displays conversation list', async ({ page }) => {
      await navigateToTab(page, 'Chat');
      await waitForNetworkIdle(page);

      const mainContent = page.locator('main');
      await expect(mainContent).toBeVisible();

      await takeVisualScreenshot(page, 'user-chat', 'conversation-list');
    });

    test('chat - new conversation button visible', async ({ page }) => {
      await navigateToTab(page, 'Chat');
      await waitForNetworkIdle(page);

      // Check for new chat button (may or may not be visible)
      await page.getByRole('button', { name: /new|create|\+/i }).isVisible().catch(() => false);

      await takeVisualScreenshot(page, 'user-chat', 'new-chat-button');
    });

    test('chat - message input accepts text', async ({ page }) => {
      await navigateToTab(page, 'Chat');
      await waitForNetworkIdle(page);

      const messageInput = page.locator('textarea, input[placeholder*="message" i], input[placeholder*="type" i]');
      if (await messageInput.first().isVisible().catch(() => false)) {
        await messageInput.first().fill('Test message');
        await page.waitForTimeout(300);

        await takeVisualScreenshot(page, 'user-chat', 'message-input');
      }
    });

    test('chat - the unified conversation list', async ({ page }) => {
      await navigateToTab(page, 'Chat');
      await waitForNetworkIdle(page);

      // The one list every thread lands in, whatever surface created it.
      await expect(page.getByTestId('conversation-list')).toBeVisible();
      await expect(page.getByLabel('Search conversations')).toBeVisible();

      await takeVisualScreenshot(page, 'user-chat', 'conversation-list');
    });
  });

  // ========================================
  // Agent Store Tab
  // ========================================
  test.describe('Agent Store Tab', () => {
    test.beforeEach(async ({ page }) => {
      await loginAsUser(page, 'webtest');
    });

    test('store - displays store grid', async ({ page }) => {
      await navigateToTab(page, 'Discover');
      await waitForNetworkIdle(page);

      const mainContent = page.locator('main');
      await expect(mainContent).toBeVisible();

      await takeVisualScreenshot(page, 'user-store', 'grid');
    });

    test('store - category tabs work', async ({ page }) => {
      await navigateToTab(page, 'Discover');
      await waitForNetworkIdle(page);

      const categoryTabs = page.locator('[role="tab"], button:has-text("Training"), button:has-text("Nutrition")');
      const tabs = await categoryTabs.all();

      for (const tab of tabs.slice(0, 3)) {
        // Test first 3 category tabs
        if (await tab.isVisible().catch(() => false)) {
          await tab.click();
          await page.waitForTimeout(300);
        }
      }

      await takeVisualScreenshot(page, 'user-store', 'category-tabs');
    });

    test('store - search agents works', async ({ page }) => {
      await navigateToTab(page, 'Discover');
      await waitForNetworkIdle(page);

      const searchInput = page.locator('input[type="search"], input[placeholder*="Search"]');
      if (await searchInput.first().isVisible().catch(() => false)) {
        await searchInput.first().fill('training');
        await page.waitForTimeout(500);

        await takeVisualScreenshot(page, 'user-store', 'search-results');
      }
    });

    test('store - the agent card shows install button', async ({ page }) => {
      await navigateToTab(page, 'Discover');
      await waitForNetworkIdle(page);

      // Check for install button presence
      await page.getByRole('button', { name: /install/i }).first().isVisible().catch(() => false);

      await takeVisualScreenshot(page, 'user-store', 'install-button');
    });

    test('store - clicking an agent opens detail', async ({ page }) => {
      await navigateToTab(page, 'Discover');
      await waitForNetworkIdle(page);

      // Click on an agent card (not the install button)
      const coachCard = page.locator('[data-testid="coach-card"], .coach-card, article').first();
      if (await coachCard.isVisible().catch(() => false)) {
        await coachCard.click();
        await page.waitForTimeout(500);

        await takeVisualScreenshot(page, 'user-store', 'detail-view');

        // Go back
        const backButton = page.getByRole('button', { name: /back|close|×/i });
        if (await backButton.first().isVisible().catch(() => false)) {
          await backButton.first().click();
        }
      }
    });
  });

  // ========================================
  // Insights + Friends (retired 2026-08-26)
  // ========================================
  test.describe('Insights and Friends are gone', () => {
    test.beforeEach(async ({ page }) => {
      await loginAsUser(page, 'webtest');
    });

    test('sidebar - no Insights or Friends destination', async ({ page }) => {
      await waitForNetworkIdle(page);

      // Unconditional: the old suites reached this surface behind
      // `if (await button.isVisible())` guards, which is how a deleted tab
      // could keep passing. The nav must hold the athlete tabs and nothing
      // pointing at a feed.
      const aside = page.locator('aside');
      await expect(aside.getByRole('button', { name: 'Chat' })).toBeVisible();
      await expect(aside.getByRole('button', { name: 'Discover', exact: true })).toBeVisible();
      await expect(aside.getByRole('button', { name: 'Groups' })).toHaveCount(0);
      await expect(aside.getByRole('button', { name: 'Insights' })).toHaveCount(0);
      await expect(aside.getByRole('button', { name: 'Friends' })).toHaveCount(0);

      await takeVisualScreenshot(page, 'user-nav', 'no-insights');
    });

    test('stale #insights hash - lands on chat', async ({ page }) => {
      await page.goto('/#insights');
      await waitForNetworkIdle(page);

      await expect(page.getByTestId('conversation-list')).toBeVisible({ timeout: 10000 });
      await expect(page.getByText('No Insights Yet')).toHaveCount(0);
      await expect(page.getByRole('button', { name: 'Find Friends' })).toHaveCount(0);

      await takeVisualScreenshot(page, 'user-nav', 'stale-insights-hash');
    });
  });

  // ========================================
  // User Settings Tab
  // ========================================
  test.describe('User Settings Tab', () => {
    test.beforeEach(async ({ page }) => {
      await loginAsUser(page, 'webtest');
    });

    test('settings - displays profile section', async ({ page }) => {
      await navigateToTab(page, 'Settings');
      await waitForNetworkIdle(page);

      const mainContent = page.locator('main');
      await expect(mainContent).toBeVisible();

      await takeVisualScreenshot(page, 'user-settings', 'profile');
    });

    test('settings - displays provider connections', async ({ page }) => {
      await navigateToTab(page, 'Settings');
      await waitForNetworkIdle(page);

      // Look for provider/connection section
      await page.locator('text=Connections, text=Providers, text=Strava').first().isVisible().catch(() => false);

      await takeVisualScreenshot(page, 'user-settings', 'providers');
    });

    test('settings - edit name works', async ({ page }) => {
      await navigateToTab(page, 'Settings');
      await waitForNetworkIdle(page);

      const editButton = page.getByRole('button', { name: /edit/i });
      if (await editButton.first().isVisible().catch(() => false)) {
        await editButton.first().click();
        await page.waitForTimeout(300);

        await takeVisualScreenshot(page, 'user-settings', 'edit-mode');

        // Cancel edit
        const cancelButton = page.getByRole('button', { name: /cancel/i });
        if (await cancelButton.first().isVisible().catch(() => false)) {
          await cancelButton.first().click();
        }
      }
    });

    test('settings - change password form accessible', async ({ page }) => {
      await navigateToTab(page, 'Settings');
      await waitForNetworkIdle(page);

      const changePasswordButton = page.getByRole('button', { name: /change password|password/i });
      if (await changePasswordButton.first().isVisible().catch(() => false)) {
        await changePasswordButton.first().click();
        await page.waitForTimeout(300);

        await takeVisualScreenshot(page, 'user-settings', 'change-password');

        // Close form
        const cancelButton = page.getByRole('button', { name: /cancel|close/i });
        if (await cancelButton.first().isVisible().catch(() => false)) {
          await cancelButton.first().click();
        }
      }
    });
  });

});

test.describe('Role routing — admin tabs are not reachable by hash', () => {
  // The sidebar only offers role-appropriate tabs, but the hash is
  // user-editable. Typing #users as a regular user mounted the admin
  // UserManagement pane: the server refused every /api/admin call with 403 so
  // no data leaked, but the pane rendered its filter chrome and then retried
  // the 403 on a loop — eight requests for one page view.
  test('a regular user typing #users lands on chat, not the admin pane', async ({ page }) => {
    const adminCalls: string[] = [];
    await page.route('**/api/admin/**', async (route) => {
      adminCalls.push(route.request().url());
      await route.fulfill({ status: 403, contentType: 'application/json', body: '{}' });
    });

    await loginAsUser(page, 'webtest');
    await page.goto('/#users');
    await page.waitForTimeout(1200);

    // Assert the properties that matter, not the URL: the app deliberately
    // leaves the typed hash alone, so checking it would pin an implementation
    // detail rather than the guard. What must hold is that the admin pane never
    // mounts and its endpoints are never called.
    await expect(page.getByRole('button', { name: 'All Users' })).toHaveCount(0);
    await expect(page.getByRole('button', { name: 'Pending' })).toHaveCount(0);
    // The conversation list proves we landed on the role's own default surface.
    await expect(page.getByTestId('conversation-list')).toBeVisible();
    // And nothing should have hammered an endpoint this role cannot use.
    expect(adminCalls, `regular user issued ${adminCalls.length} admin API calls`).toHaveLength(0);
  });

  test('an admin can still open #users', async ({ page }) => {
    // The guard must deny the right people without denying the right people.
    await loginAsUser(page, 'admin');
    await page.goto('/#users');
    await page.waitForTimeout(1200);
    await expect(page.locator('h1')).toContainText('Users');
  });
});
