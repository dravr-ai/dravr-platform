// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: E2E for the feedback thread with the Dravr team — the account menu opens it, a message carries its context or a screenshot, a team reply dots it
// ABOUTME: Opened from a thumbs-down link it carries that reply; runs against a mocked /api/me/feedback shaped like the server's (carnet#1235, #1392)

import { test, expect, type Page } from '@playwright/test';
import { APP_VERSION } from '@pierre/shared-constants';
import { setupDashboardMocks, loginToDashboard } from './test-helpers';

/** Every POST the thread sent, as the server received it. */
type SentBodies = Array<Record<string, unknown>>;

/** A real 1×1 PNG, so the browser has to decode what the thread shows. */
const SCREENSHOT_PNG = Buffer.from(
  'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAIAAACQd1PeAAAADElEQVR4nGP4z8AAAAMBAQDJ/pLvAAAAAElFTkSuQmCC',
  'base64',
);

/**
 * Serve the athlete's thread: `messages` at first (empty by default), then
 * each POST answered with the stored message the server would return.
 * Registered after the shared stubs, so these answers win.
 */
async function mockFeedbackThread(page: Page, messages: Array<Record<string, unknown>> = []): Promise<SentBodies> {
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
            author: 'athlete',
            body: body.body,
            created_at: new Date().toISOString(),
            about: body.about ?? null,
            has_image: body.image !== undefined,
          },
        }),
      });
      return;
    }
    await route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify({ messages }) });
  });
  return sent;
}

/**
 * Serve the unread count as `unread` team replies until the thread is marked
 * read, which zeroes it as the server does; returns how often it was marked.
 */
async function mockUnreadReplies(page: Page, unread: number): Promise<{ reads: number }> {
  const state = { unread, reads: 0 };
  await page.route('**/api/me/feedback/unread', async (route) => {
    await route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify({ unread: state.unread }) });
  });
  await page.route('**/api/me/feedback/read', async (route) => {
    state.reads += 1;
    const marked = state.unread;
    state.unread = 0;
    await route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify({ marked }) });
  });
  return {
    get reads() {
      return state.reads;
    },
  };
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

  test('a screenshot picked from the composer goes out alone as base64, then shows in the thread', async ({ page }) => {
    await setupDashboardMocks(page, { role: 'user' });
    const sent = await mockFeedbackThread(page);
    const imageReads: string[] = [];
    await page.route('**/api/me/feedback/*/image', async (route) => {
      imageReads.push(new URL(route.request().url()).pathname);
      await route.fulfill({ status: 200, contentType: 'image/png', body: SCREENSHOT_PNG });
    });
    await loginToDashboard(page);

    await page.getByRole('button', { name: 'Account menu' }).click();
    await page.getByRole('button', { name: 'Send feedback' }).click();
    await page.waitForURL(/#feedback$/);

    const thread = page.getByTestId('feedback-thread');
    const chooser = page.waitForEvent('filechooser');
    await thread.getByRole('button', { name: 'Attach a screenshot' }).click();
    await (await chooser).setFiles({ name: 'shot.png', mimeType: 'image/png', buffer: SCREENSHOT_PNG });
    await expect(thread.getByTestId('feedback-image-preview')).toBeVisible();

    await thread.getByRole('button', { name: 'Send', exact: true }).click();

    await expect.poll(() => sent).toHaveLength(1);
    expect(sent[0]).toEqual({
      body: '',
      screen: 'home',
      app_version: APP_VERSION,
      locale: 'en',
      image: { media_type: 'image/png', data: SCREENSHOT_PNG.toString('base64') },
    });
    await expect(thread.getByTestId('feedback-image-preview')).toHaveCount(0);

    const shown = thread.getByRole('img', { name: 'Screenshot you sent' });
    await expect(shown).toBeVisible();
    // Decoded from the bytes the image route served, through the API client's blob.
    await expect.poll(() => shown.evaluate((element: HTMLImageElement) => element.naturalWidth)).toBe(1);
    expect(imageReads).toEqual(['/api/me/feedback/m-1/image']);
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

  test('a reply from the team dots the account menu, shows in the thread as the team, and is marked read', async ({ page }) => {
    await setupDashboardMocks(page, { role: 'user' });
    await mockFeedbackThread(page, [
      {
        id: 'm-1',
        author: 'athlete',
        body: 'The week view skips my indoor ride.',
        created_at: '2026-10-28T15:00:00Z',
        about: null,
        has_image: false,
      },
      {
        id: 'm-2',
        author: 'team',
        body: 'Fixed in tonight’s build.',
        created_at: '2026-10-28T16:00:00Z',
        about: null,
        has_image: false,
      },
    ]);
    const unread = await mockUnreadReplies(page, 1);
    await loginToDashboard(page);

    await expect(page.getByTestId('feedback-unread-dot')).toBeVisible();
    await page.getByRole('button', { name: 'Account menu, 1 unread' }).click();
    await page.getByRole('button', { name: 'Send feedback 1 unread' }).click();
    await page.waitForURL(/#feedback$/);

    await expect(page.getByTestId('feedback-thread').getByText('Fixed in tonight’s build.')).toBeVisible();
    await expect.poll(() => unread.reads).toBe(1);
    await expect(page.getByTestId('feedback-unread-dot')).toHaveCount(0);
  });
});
