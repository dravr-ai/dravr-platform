// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Mobile-viewport smoke tests for the authenticated shell.
// ABOUTME: Verifies bottom tab bar, drawer, no horizontal overflow, tap targets.

import { test, expect, type Page } from '@playwright/test';
import { setupDashboardMocks, loginToDashboard } from './test-helpers';

const CONVERSATION = {
  id: 'conv-mobile-1',
  title: 'Tempo intervals',
  coach_id: null,
  created_at: '2026-08-20T10:00:00Z',
  updated_at: '2026-08-20T10:00:00Z',
  message_count: 0,
  unread_count: 0,
  last_message: null,
};

/** Serves the conversation the "+" creates, and its (empty) transcript. */
async function mockConversationCreate(page: Page) {
  await page.route(`**/api/chat/conversations/${CONVERSATION.id}/messages`, async (route) => {
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({ messages: [] }),
    });
  });
  await page.route(/\/api\/chat\/conversations(\?.*)?$/, async (route, request) => {
    if (request.method() === 'POST') {
      await route.fulfill({
        status: 201,
        contentType: 'application/json',
        body: JSON.stringify(CONVERSATION),
      });
      return;
    }
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({
        conversations: [CONVERSATION],
        total: 1,
        limit: 50,
        offset: 0,
      }),
    });
  });
}

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

  test('bottom tab bar renders with 4 entries including Menu', async ({ page }) => {
    const nav = page.getByRole('navigation', { name: 'Primary navigation' });
    await expect(nav).toBeVisible();
    // 3 primary + Menu. Insights was retired by the Chat-First Cutover, the
    // Coach tab folded into Discover, and Groups moved inside the group's own
    // chat thread — so the bar holds exactly these and nothing else.
    await expect(nav.getByRole('button', { name: 'Chat' })).toBeVisible();
    await expect(nav.getByRole('button', { name: 'Discover' })).toBeVisible();
    await expect(nav.getByRole('button', { name: 'Notifications' })).toBeVisible();
    await expect(nav.getByRole('button', { name: 'Open menu' })).toBeVisible();
    await expect(nav.getByRole('button')).toHaveCount(4);
    await expect(nav.getByRole('button', { name: 'Insights' })).toHaveCount(0);
    await expect(nav.getByRole('button', { name: 'Coaches' })).toHaveCount(0);
    await expect(nav.getByRole('button', { name: 'Groups' })).toHaveCount(0);
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

  test('navigating from drawer dismisses it and switches tab', async ({ page }) => {
    // Chat, Discover and Notifications are the pinned primary tabs; a
    // provider connection is configuration, so it lives under Settings and
    // the drawer offers Settings, not a Data Providers destination.
    await page.getByRole('button', { name: 'Open menu' }).click();
    const drawer = page.getByRole('dialog', { name: 'Secondary navigation' });
    await expect(drawer.getByRole('button', { name: /^Data Providers/ })).toHaveCount(0);
    await drawer.getByRole('button', { name: 'Settings' }).click();
    await expect(drawer).toBeHidden();
    await expect(page).toHaveURL(/#settings/);
  });
});

test.describe('Mobile composer', () => {
  // The composer belongs to an open thread — the empty pane offers the "+" and
  // Commands instead. The conversation list lives in the desktop sidebar, so at
  // this width the "+" is the athlete's way into a thread; taking it is what
  // puts the composer, and its tap target, on screen.
  test.beforeEach(async ({ page }) => {
    await setupDashboardMocks(page, {
      role: 'user',
      email: 'alice@acme.com',
      displayName: 'Alice Test',
    });
    await mockConversationCreate(page);
    await loginToDashboard(page, { email: 'alice@acme.com', password: 'password123' });
    await page.getByTestId('conversation-pane').getByRole('button', { name: 'New', exact: true }).click();
    await page.getByRole('menuitem', { name: 'New chat' }).click();
    await expect(page.getByPlaceholder('Message Dravr...').first()).toBeVisible();
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
});
