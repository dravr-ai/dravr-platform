// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: E2E for the feedback thread with the Dravr team — the account menu opens it, and a sent message carries its context
// ABOUTME: Opened from a thumbs-down link it carries that reply; runs against a mocked /api/me/feedback shaped like the server's (carnet#1235)

import { test, expect, type Page } from '@playwright/test';
import { APP_VERSION } from '@pierre/shared-constants';
import { setupDashboardMocks, loginToDashboard } from './test-helpers';

/** Every POST the thread sent, as the server received it. */
type SentBodies = Array<Record<string, unknown>>;

/**
 * Serve the athlete's thread: empty at first, then each POST answered with
 * the stored message the server would return. Registered after the shared
 * stubs, so these answers win.
 */
async function mockFeedbackThread(page: Page): Promise<SentBodies> {
  const sent: SentBodies = [];
  await page.route('**/api/me/feedback', async (route) => {
    const request = route.request();
    if (request.method() === 'POST') {
      const body = request.postDataJSON() as Record<string, unknown>;
      sent.push(body);
      await route.fulfill({
        status: 201,
        contentType: 'application/json',
        body: JSON.stringify({
          message: {
            id: `m-${sent.length}`,
            body: body.body,
            created_at: new Date().toISOString(),
            about: body.about ?? null,
          },
        }),
      });
      return;
    }
    await route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify({ messages: [] }) });
  });
  return sent;
}

test.describe('Feedback thread', () => {
  test('the account menu opens the thread, and a message goes out with its screen, version and language', async ({ page }) => {
    await setupDashboardMocks(page, { role: 'user' });
    const sent = await mockFeedbackThread(page);
    await loginToDashboard(page);

    await page.getByRole('button', { name: 'Account menu' }).click();
    await page.getByRole('button', { name: 'Send feedback' }).click();
    await page.waitForURL(/#feedback$/);
    await expect(page.getByRole('heading', { name: 'Dravr team' })).toBeVisible();

    await page.getByTestId('feedback-thread').getByRole('textbox', { name: 'Message the Dravr team...' }).fill('The week view skips my indoor ride.');
    await page.getByTestId('feedback-thread').getByRole('button', { name: 'Send', exact: true }).click();

    await expect.poll(() => sent).toHaveLength(1);
    expect(sent[0]).toEqual({
      body: 'The week view skips my indoor ride.',
      screen: 'home',
      app_version: APP_VERSION,
      locale: 'en',
    });
    await expect(page.getByTestId('feedback-thread').getByText('The week view skips my indoor ride.')).toBeVisible();
  });

  test('opened from a thumbs-down link, the message carries that reply once', async ({ page }) => {
    await setupDashboardMocks(page, { role: 'user' });
    const sent = await mockFeedbackThread(page);
    await loginToDashboard(page);

    await page.evaluate(() => {
      window.location.hash = '#feedback/reply/conv-1/msg-1';
    });
    await expect(page.getByTestId('feedback-about')).toHaveText('About an agent reply');

    await page.getByTestId('feedback-thread').getByRole('textbox', { name: 'Message the Dravr team...' }).fill('Tuesday is counted twice.');
    await page.getByTestId('feedback-thread').getByRole('button', { name: 'Send', exact: true }).click();

    await expect.poll(() => sent).toHaveLength(1);
    expect(sent[0].about).toEqual({ conversation_id: 'conv-1', message_id: 'msg-1' });
    await expect(page.getByTestId('feedback-about')).toHaveCount(0);
    await page.waitForURL(/#feedback$/);
  });
});
