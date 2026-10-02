// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: E2E for the coach's onboarding group step — name the group, leave with its invite link and QR code
// ABOUTME: The first spec on the coach branch: a coach with and without coach access, and putting the step off

import { test, expect, type Page, type Route } from '@playwright/test';
import { setupDashboardMocks } from './test-helpers';

const json = (route: Route, status: number, body: unknown) =>
  route.fulfill({ status, contentType: 'application/json', body: JSON.stringify(body) });

/**
 * A connected coach whose agent step is done, so the flow opens on the group
 * step. `managesRoster` decides whether the created group names them its coach.
 */
async function setupCoach(page: Page, managesRoster: boolean) {
  const calls = { createGroup: [] as unknown[], invites: [] as unknown[], steps: [] as string[] };

  await page.addInitScript(() => {
    window.localStorage.setItem('dravr.profile_type_chosen.user-123', '1');
    window.localStorage.setItem('dravr.coach_proposal_done.user-123', '1');
    window.localStorage.setItem('dravr.theme', 'dark');
  });
  await setupDashboardMocks(page, { role: 'user', email: 'coach@test.com', displayName: 'Coach' });

  // Registered after the dashboard defaults, so these win.
  await page.route('**/api/me/onboarding-status', (route) =>
    json(route, 200, {
      needs_provider_connection: false,
      steps: [],
      chosen_channel: null,
      coaches_others: true,
    }),
  );
  await page.route('**/api/me/onboarding/steps/**', async (route) => {
    calls.steps.push(`${route.request().url().split('/').pop()}:${route.request().postDataJSON().status}`);
    await route.fulfill({ status: 204, body: '' });
  });
  await page.route('**/api/groups', async (route) => {
    calls.createGroup.push(route.request().postDataJSON());
    await json(route, 201, {
      id: 'g-1',
      tenant_id: 'user-123',
      name: 'Les Rouleurs',
      description: null,
      agent_id: 'a-1',
      owner_id: 'user-123',
      coach_user_id: managesRoster ? 'user-123' : null,
    });
  });
  await page.route('**/api/groups/g-1/invites', async (route) => {
    calls.invites.push(route.request().postDataJSON());
    await json(route, 201, { id: 'i-1', group_id: 'g-1', code: 'ABCD2345', kind: 'member' });
  });
  await page.route('**/api/chat/conversations', async (route) => {
    if (route.request().method() !== 'POST') return route.fallback();
    await json(route, 201, { id: 'c-1', title: 'Les Rouleurs', group_id: 'g-1' });
  });

  await page.goto('/');
  await page.waitForSelector('form', { timeout: 10_000 });
  await page.locator('input[name="email"]').fill('coach@test.com');
  await page.locator('input[name="password"]').fill('password123');
  await page.getByRole('button', { name: 'Sign in' }).click();
  await expect(page.getByTestId('onboarding-group-name')).toBeVisible({ timeout: 15_000 });

  return calls;
}

test('a coach with coach access leaves onboarding with a group, its link and its QR code', async ({
  page,
}) => {
  const calls = await setupCoach(page, true);

  await page.getByTestId('onboarding-group-name').fill('Les Rouleurs');
  await page.getByRole('button', { name: 'Create the group' }).click();

  await expect(page.getByTestId('onboarding-group-link')).toContainText('/groups/join/ABCD2345');
  await expect(page.getByTestId('onboarding-group-qr')).toBeVisible();
  await expect(page.getByTestId('onboarding-group-access-pending')).toHaveCount(0);
  expect(calls.createGroup).toEqual([{ name: 'Les Rouleurs', coach_is_me: true }]);
  expect(calls.invites).toEqual([{ expires_in_days: 30 }]);

  await page.getByRole('button', { name: 'Go to my group' }).click();
  await expect(page.getByTestId('onboarding-flow')).toHaveCount(0, { timeout: 10_000 });
  expect(calls.steps).toContain('coach_group:complete');
});

test('a coach without coach access gets the group and is told access is pending', async ({
  page,
}) => {
  await setupCoach(page, false);

  await page.getByTestId('onboarding-group-name').fill('Les Rouleurs');
  await page.getByRole('button', { name: 'Create the group' }).click();

  await expect(page.getByTestId('onboarding-group-access-pending')).toContainText(
    'Coach access pending',
  );
  await expect(page.getByRole('link', { name: 'Ask at support@dravr.ai' })).toHaveAttribute(
    'href',
    'mailto:support@dravr.ai',
  );
});

test('a coach can put the group step off', async ({ page }) => {
  const calls = await setupCoach(page, false);

  await page.getByRole('button', { name: 'Later' }).click();
  await expect(page.getByTestId('onboarding-flow')).toHaveCount(0, { timeout: 10_000 });
  expect(calls.steps).toContain('coach_group:skipped');
  expect(calls.createGroup).toEqual([]);
});
