// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: E2E for the messaging onboarding steps — pick a chat app then connect it via QR, poll-to-complete
// ABOUTME: A connected user with the agent step done + channels configured lands on the messaging steps, then reaches the dashboard

import { test, expect, type Page } from '@playwright/test';
import { setupDashboardMocks } from './test-helpers';

/** Wire the messaging-onboarding mocks. `links` starts empty and flips to linked. */
function setupMessagingMocks(page: Page) {
  const state = { linked: false };
  const stub = async (route: import('@playwright/test').Route, body: unknown) =>
    route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify(body) });

  return {
    async install() {
      await page.route('**/api/messaging/channels/available', (route) =>
        stub(route, [
          { channel: 'telegram', display_name: 'Telegram', method: 'deep_link', recommended: true },
          { channel: 'slack', display_name: 'Slack', method: 'oauth', recommended: false },
        ]),
      );
      await page.route('**/api/messaging/link/init/**', (route) =>
        stub(route, {
          channel: 'telegram',
          method: 'deep_link',
          code: 'abc123',
          linking_url: 'https://t.me/DravrBot?start=abc123',
          expires_at: '2030-01-01T00:00:00Z',
          qr_svg: '<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"></svg>',
        }),
      );
      await page.route('**/api/messaging/links', (route) =>
        stub(route, {
          links: state.linked
            ? [{ channel: 'telegram', channel_user_id: 'tg1', display_name: null, linked_at: 'now' }]
            : [],
        }),
      );
      // The onboarding step PUT — accept and ignore.
      await page.route('**/api/me/onboarding/steps/**', (route) =>
        route.fulfill({ status: 204, body: '' }),
      );
    },
    completeLink() {
      state.linked = true;
    },
  };
}

test('messaging onboarding: pick a channel, connect via QR, auto-advance to the dashboard', async ({
  page,
}) => {
  // A connected user (needs_provider_connection: false via setupDashboardMocks)
  // with profile + agent already done → the flow opens on the messaging steps.
  await page.addInitScript(() => {
    window.localStorage.setItem('dravr.profile_type_chosen.user-123', '1');
    window.localStorage.setItem('dravr.coach_proposal_done.user-123', '1');
    window.localStorage.setItem('dravr.theme', 'dark');
  });

  await setupDashboardMocks(page, { role: 'user', email: 'msg@test.com', displayName: 'Msg User' });
  const mocks = setupMessagingMocks(page);
  await mocks.install();

  await page.goto('/');
  await page.waitForSelector('form', { timeout: 10_000 });
  await page.locator('input[name="email"]').fill('msg@test.com');
  await page.locator('input[name="password"]').fill('password123');
  await page.getByRole('button', { name: 'Sign in' }).click();

  // Step: messaging channel picker (two channels configured → the picker shows).
  await expect(page.getByText(/Where do you want to chat with your agent/i)).toBeVisible({
    timeout: 15_000,
  });
  await expect(page.getByText('Telegram')).toBeVisible();
  await expect(page.getByText('Slack')).toBeVisible();
  await expect(page.getByText('Recommended')).toBeVisible();

  // Pick Telegram → the configure step with a QR + open button (deep-link channel).
  await page.getByText('Telegram').click();
  await expect(page.getByRole('img', { name: /QR code to connect Telegram/i })).toBeVisible({
    timeout: 15_000,
  });
  await expect(page.getByRole('button', { name: 'Open Telegram' })).toBeVisible();

  // The account link lands → the poll picks it up → we auto-advance to the dashboard.
  mocks.completeLink();
  await expect(page.locator('main')).toBeVisible({ timeout: 15_000 });
});
