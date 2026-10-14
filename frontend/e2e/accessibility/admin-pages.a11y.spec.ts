// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Scans each admin console page for the defects a Lighthouse run found on them: unnamed controls, skipped headings
// ABOUTME: Each page first proves it drew the controls the scan is about, so a mock that renders nothing cannot pass

import { test, expect, type Page } from '@playwright/test';
import AxeBuilder from '@axe-core/playwright';
import { loginAsUser, navigateToTab } from '../visual-test-helpers';

/**
 * The axe rules behind the Lighthouse failures: a switch with no name, a
 * checkbox or select with no label, a page jumping from its H1 to an H3, and
 * a control whose name is not the text it shows (WCAG 2.5.3).
 */
const RULES = ['button-name', 'label', 'select-name', 'heading-order', 'label-content-name-mismatch', 'empty-heading'];

async function mockAdminSettings(page: Page) {
  await page.route('**/api/admin/settings/auto-approval', (route) =>
    route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({
        data: { enabled: false, auto_approve_domains: [], overridden_by_env: false, description: 'Auto-approval disabled' },
      }),
    }),
  );
  await page.route('**/api/groups/permissions', (route) =>
    route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({ can_create: true, policy: 'admins_only', weekly_digest_enabled: false }),
    }),
  );
  // Registered after the dashboard's empty list, so it answers first: the
  // Users page draws its pending section — the headings under test — only
  // when someone is waiting.
  await page.route('**/api/admin/pending-users', (route) =>
    route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({
        success: true,
        message: 'Retrieved 1 pending users',
        data: {
          count: 1,
          users: [
            {
              id: 'pending-1',
              email: 'quinn@example.com',
              display_name: 'Quinn Roberts',
              user_status: 'pending',
              tier: 'starter',
              created_at: '2026-10-13T09:00:00Z',
            },
          ],
        },
      }),
    }),
  );
  await page.route('**/api/admin/contremaitre/status', (route) =>
    route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({
        configured: true,
        repo: 'dravr-ai/dravr-contremaitre',
        branch: 'main',
        system_prompt_count: 2,
        agent_prompt_count: 3,
        compiled_in_count: 0,
        contremaitre_count: 5,
      }),
    }),
  );
  await page.route('**/api/admin/contremaitre/prompts', (route) =>
    route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({
        total: 2,
        prompts: [
          { key: 'coaching_persona', sha256: 'a1', source: 'contremaitre', loaded_at: '2026-10-14T12:00:00Z', content_length: 1200 },
          { key: 'turn_directive', sha256: 'b2', source: 'compiled_in', loaded_at: '2026-10-14T12:00:00Z', content_length: 800 },
        ],
      }),
    }),
  );
}

async function scan(page: Page, route: string) {
  const results = await new AxeBuilder({ page }).include('main').withRules(RULES).analyze();
  const summary = results.violations.map((v) => ({ rule: v.id, nodes: v.nodes.map((n) => n.target.join(' ')) }));
  expect({ route, violations: summary }).toEqual({ route, violations: [] });
}

test.describe('Admin console pages — accessible names and heading order', () => {
  test.beforeEach(async ({ page }) => {
    await loginAsUser(page, 'admin');
    await mockAdminSettings(page);
  });

  test('Tool Management names every switch and checkbox', async ({ page }) => {
    await navigateToTab(page, 'Tool Management');
    await expect(page.getByRole('switch').first()).toBeVisible();
    await expect(page.getByRole('checkbox').first()).toBeVisible();
    await expect(page.getByRole('switch', { name: 'Toggle Get Activities' })).toBeVisible();
    await scan(page, 'configuration');
  });

  test('Platform Settings names its switch and select', async ({ page }) => {
    await navigateToTab(page, 'Platform Settings');
    await expect(page.getByRole('switch', { name: 'Auto-Approve Registrations' })).toBeVisible();
    await expect(page.getByRole('combobox', { name: 'Group Creation Policy' })).toBeVisible();
    await scan(page, 'platform-settings');
  });

  test('Users steps down from the page heading one level at a time', async ({ page }) => {
    // Sign-in lands on Users and read the dashboard's empty list; leave and
    // come back so the page reads the pending user mocked above.
    await navigateToTab(page, 'Prompts');
    await navigateToTab(page, 'Users');
    await expect(page.getByRole('heading', { level: 2, name: /Pending Users/ })).toBeVisible();
    await expect(page.getByRole('heading', { level: 3, name: 'Quinn Roberts' })).toBeVisible();
    await scan(page, 'users');
  });

  test('Prompts steps down from the page heading one level at a time', async ({ page }) => {
    await navigateToTab(page, 'Prompts');
    await expect(page.getByRole('heading', { level: 2, name: 'System Prompts' })).toBeVisible();
    await expect(page.getByRole('heading', { level: 3 }).first()).toBeVisible();
    await scan(page, 'prompts');
  });

  test('the sidebar user button is named by the name it shows', async ({ page }) => {
    const userButton = page.locator('aside').getByRole('button', { name: /Admin/ }).first();
    await expect(userButton).toBeVisible();
    const results = await new AxeBuilder({ page }).include('aside').withRules(['label-content-name-mismatch']).analyze();
    expect(results.violations).toEqual([]);
  });
});
