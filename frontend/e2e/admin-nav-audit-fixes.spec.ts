// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Playwright E2E coverage for the 2026-05-07 admin nav audit fixes
// ABOUTME: Locks every fix from the audit gist 56c1c1d7b54bf16c9d140f54f3210c94

import { test, expect } from '@playwright/test';
import { setupAndLoginAsAdmin, navigateToTab } from './test-helpers';

test.describe('Admin nav audit fixes (gist 56c1c1d7)', () => {
  test('Service Tokens page renders without 500 (audit #19)', async ({ page }) => {
    await page.route('**/api/admin/tokens**', (r) =>
      r.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({ tokens: [] }),
      }),
    );
    await setupAndLoginAsAdmin(page);
    await navigateToTab(page, 'Service Tokens');
    await expect(page.getByRole('heading', { name: 'Service Tokens' })).toBeVisible();
    await expect(page.getByRole('button', { name: /Create API Token/ })).toBeVisible();
  });

  test('Engagement page enforces DAU ≤ WAU ≤ MAU (audit #17)', async ({ page }) => {
    await setupAndLoginAsAdmin(page);
    const now = Date.now();
    const day = 86_400_000;
    // Override AFTER setup so our handler runs first (Playwright evaluates
    // routes in reverse registration order). getAllUsers() unwraps
    // `response.data.users`, so the body shape is `{ users: [...] }`.
    await page.route('**/api/admin/users**', (r) =>
      r.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({
          users: [
            { id: 'u1', email: 'u1@x', last_active: new Date(now - 0.5 * day).toISOString() },
            { id: 'u2', email: 'u2@x', last_active: new Date(now - 0.5 * day).toISOString() },
            { id: 'u3', email: 'u3@x', last_active: new Date(now - 3 * day).toISOString() },
            { id: 'u4', email: 'u4@x', last_active: new Date(now - 20 * day).toISOString() },
          ],
          total_count: 4,
        }),
      }),
    );
    await page.route('**/api/dashboard/analytics**', (r) =>
      r.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({
          time_series: [],
          top_tools: [],
          error_rate: 0,
          average_response_time: 0,
        }),
      }),
    );
    await navigateToTab(page, 'Engagement');

    const card = (label: string) =>
      page.locator('div', { has: page.locator(`text=${label}`) }).locator('.text-2xl').first();
    const dau = Number(await card('Daily Active').innerText());
    const wau = Number(await card('Weekly Active').innerText());
    const mau = Number(await card('Monthly Active').innerText());
    expect(dau).toBeLessThanOrEqual(wau);
    expect(wau).toBeLessThanOrEqual(mau);
  });

  test('Engagement Coach Leaderboard column header reads "Prompt Size" (audit #17)', async ({
    page,
  }) => {
    await setupAndLoginAsAdmin(page);
    // Need at least one system coach so the leaderboard table renders.
    await page.route('**/api/admin/coaches**', (r) =>
      r.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({
          coaches: [
            {
              id: 'coach-1',
              title: 'Marathon Coach',
              category: 'training',
              token_count: 392,
              use_count: 0,
              author_user_id: 'user-123',
              status: 'active',
            },
          ],
        }),
      }),
    );
    await page.route('**/api/admin/users**', (r) =>
      r.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({ users: [], total_count: 0 }),
      }),
    );
    await page.route('**/api/dashboard/analytics**', (r) =>
      r.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({
          time_series: [],
          top_tools: [],
          error_rate: 0,
          average_response_time: 0,
        }),
      }),
    );
    await navigateToTab(page, 'Engagement');
    await expect(page.getByRole('columnheader', { name: /Prompt Size/i })).toBeVisible();
    await expect(page.getByRole('columnheader', { name: /^Tokens Used$/ })).toHaveCount(0);
  });

  test('Notifications page renders only one h1 (audit #18)', async ({ page }) => {
    await setupAndLoginAsAdmin(page);
    await navigateToTab(page, 'Notifications');
    const h1Count = await page.locator('main h1', { hasText: 'Notifications' }).count();
    expect(h1Count).toBe(1);
  });

  test('Coach Notes Audit help references coach_note_add not memory.write_note (audit #12)', async ({
    page,
  }) => {
    await page.route('**/api/admin/coach-notes**', (r) =>
      r.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({ notes: [] }),
      }),
    );
    await setupAndLoginAsAdmin(page);
    await navigateToTab(page, 'Coach Notes Audit');
    await expect(page.getByText('coach_note_add', { exact: true })).toBeVisible();
    await expect(page.getByText('memory.write_note')).toHaveCount(0);
  });

  test('Auto-approve toggle has only one description paragraph (audit #7)', async ({ page }) => {
    await page.route('**/api/admin/auto-approval', (r) =>
      r.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({
          enabled: false,
          description: 'When enabled, all new registrations are auto-approved.',
        }),
      }),
    );
    await setupAndLoginAsAdmin(page);
    await navigateToTab(page, 'Platform Settings');
    const heading = page.getByRole('heading', { name: 'Auto-Approve Registrations' });
    await expect(heading).toBeVisible();
    // The card containing the toggle should only have ONE <p> describing the
    // setting now (audit flagged two stacked paragraphs).
    const card = heading.locator('xpath=ancestor::div[contains(@class,"flex-1")][1]');
    expect(await card.locator('p').count()).toBe(1);
  });

  test('Nav click updates URL hash (audit cross-cutting: deep links)', async ({ page }) => {
    await setupAndLoginAsAdmin(page);
    await navigateToTab(page, 'Coaches');
    await expect(page).toHaveURL(/#coaches$/);
    await navigateToTab(page, 'Activity');
    await expect(page).toHaveURL(/#activity$/);
    // Reload preserves the section.
    await page.reload();
    await expect(page).toHaveURL(/#activity$/);
  });

  test('Coach Store shows "Published / Total" when counts differ (audit #3)', async ({ page }) => {
    await setupAndLoginAsAdmin(page);
    await page.route('**/api/admin/store/stats', (r) =>
      r.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({
          pending_count: 0,
          published_count: 20,
          rejected_count: 0,
          total_installs: 0,
          rejection_rate: 0,
        }),
      }),
    );
    await page.route('**/api/admin/coaches**', (r) =>
      r.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({
          coaches: Array.from({ length: 21 }, (_, i) => ({
            id: `coach-${i}`,
            title: `Coach ${i}`,
            category: 'training',
            token_count: 100 + i,
            use_count: 0,
            author_user_id: 'user-123',
            status: 'active',
          })),
        }),
      }),
    );
    await navigateToTab(page, 'Coach Store');
    await expect(page.getByText('Published / Total Coaches')).toBeVisible();
    await expect(page.getByText(/\/ 21$/)).toBeVisible();
  });

  test('Harness Config window tokens has explicit max attribute (audit #9)', async ({ page }) => {
    await page.route('**/api/admin/settings/harness', (r) =>
      r.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({
          compaction: {
            window_tokens: 128_000,
            warn_threshold: 0.7,
            emergency_threshold: 0.95,
            summarize_oldest_n: 6,
            sliding_drop_n: 4,
          },
          guardrails: {
            max_response_chars: 5_000,
            blocked_topics: [],
            disclaimer_triggers: [],
            disclaimer_text: '',
          },
          source: 'defaults',
        }),
      }),
    );
    await setupAndLoginAsAdmin(page);
    await navigateToTab(page, 'Harness Config');
    const windowTokens = page.locator('input[type="number"]').first();
    const max = await windowTokens.getAttribute('max');
    const min = await windowTokens.getAttribute('min');
    expect(Number(min)).toBeGreaterThan(0);
    expect(Number(max)).toBeGreaterThan(Number(min));
  });

  test('Memory Worker breakdown percentages sum to 100 (audit #10)', async ({ page }) => {
    await setupAndLoginAsAdmin(page);
    await page.route('**/api/admin/memory/worker-metrics**', (r) =>
      r.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({
          tenant_id: 'user-123',
          metrics: {
            total_facts: 17,
            facts_last_24h: 0,
            facts_last_7d: 5,
            distinct_users: 2,
            last_extracted_at: new Date(Date.now() - 3 * 86_400_000).toISOString(),
            facts_by_kind: {
              equipment: 6,
              goal: 4,
              preference: 4,
              other: 3,
            },
          },
        }),
      }),
    );
    await navigateToTab(page, 'Memory Worker');
    const pcts = await page.getByText(/\(\d+%\)/).allTextContents();
    const sum = pcts
      .map((t) => Number(/\((\d+)%\)/.exec(t)?.[1] ?? 0))
      .reduce((a, b) => a + b, 0);
    expect(sum).toBe(100);
  });

  test('Analytics avg response time renders the wired value (audit #20)', async ({ page }) => {
    await setupAndLoginAsAdmin(page);
    await page.route('**/api/dashboard/analytics**', (r) =>
      r.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({
          time_series: [
            {
              date: '2026-05-07',
              request_count: 4,
              error_count: 0,
              average_response_time: 12345.0,
            },
          ],
          top_tools: [],
          error_rate: 0,
          average_response_time: 12345.0,
        }),
      }),
    );
    await navigateToTab(page, 'Analytics');
    await expect(page.getByText(/12345ms/).first()).toBeVisible();
    await expect(page.getByText(/^0ms$/)).toHaveCount(0);
  });
});
