// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Pins the thread geometry only a laid-out browser shows — a short athlete bubble on one line
// ABOUTME: and a composer that grows with the draft, keeps its buttons on the last line, then scrolls

import { test, expect, type Page } from '@playwright/test';
import { setupDashboardMocks, loginToDashboard } from './test-helpers';

const CONVERSATIONS = {
  conversations: [
    {
      id: 'conv-geometry',
      title: 'Planifions ta saison',
      agent_id: 'coach-endurance',
      agent_name: 'Endurance Agent',
      created_at: '2026-09-30T15:00:00Z',
      updated_at: '2026-09-30T15:06:00Z',
      message_count: 2,
      unread_count: 0,
    },
  ],
  total: 1,
  limit: 50,
  offset: 0,
};

const MESSAGES = {
  messages: [
    {
      id: 'geometry-1',
      conversation_id: 'conv-geometry',
      role: 'user',
      content: '/season',
      created_at: '2026-09-30T15:06:00Z',
    },
    {
      id: 'geometry-2',
      conversation_id: 'conv-geometry',
      role: 'assistant',
      content: 'Planifions ta saison. Je vais te poser six courtes questions, une à la fois.',
      created_at: '2026-09-30T15:06:10Z',
      model: 'claude-sonnet-5',
      execution_time_ms: 700,
    },
  ],
};

async function openThread(page: Page) {
  await setupDashboardMocks(page, { role: 'user' });
  await page.route('**/api/chat/conversations/*/messages**', async (route) => {
    await route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify(MESSAGES) });
  });
  // The list pattern also matches the messages and read-marker URLs; those
  // fall through to the route above and to the shared mocks.
  await page.route('**/api/chat/conversations**', async (route, request) => {
    const url = request.url();
    if (url.includes('/messages') || url.includes('/read')) {
      await route.fallback();
      return;
    }
    await route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify(CONVERSATIONS) });
  });
  await loginToDashboard(page);
  await page.goto('/#chat/conv-geometry');
  await expect(page.getByTestId('message-row').first()).toBeVisible();
}

test.describe('thread geometry', () => {
  test('a short athlete message stays on one line', async ({ page }) => {
    await openThread(page);
    const words = page.locator('[data-role="user"] .chat-bubble-user p', { hasText: '/season' });
    await expect(words).toBeVisible();
    const lineHeight = await words.evaluate((el) => parseFloat(getComputedStyle(el).lineHeight));
    const box = await words.boundingBox();
    expect(box, 'the /season paragraph has a box').not.toBeNull();
    expect(box!.height).toBeLessThan(lineHeight * 1.5);
  });

  test('the composer grows with the draft, then scrolls', async ({ page }) => {
    await openThread(page);
    const composer = page.getByPlaceholder('Message Dravr...').first();
    const send = page.getByRole('button', { name: 'Send message' });
    const oneLine = (await composer.boundingBox())!.height;

    await composer.fill(
      'Je ferai le VTXL par moi même à la fin juin ou au début juillet. Je compte aussi faire Gravelooza 150 km le mois suivant, puis une course plus courte en septembre.',
    );
    const grown = (await composer.boundingBox())!;
    expect(grown.height).toBeGreaterThan(oneLine + 10);
    const sendBox = (await send.boundingBox())!;
    expect(sendBox.y + sendBox.height).toBeLessThanOrEqual(grown.y + grown.height);
    expect(grown.y + grown.height - (sendBox.y + sendBox.height)).toBeLessThan(12);

    await composer.fill(Array.from({ length: 20 }, (_, i) => `ligne ${i + 1}`).join('\n'));
    const capped = await composer.evaluate((el: HTMLTextAreaElement) => {
      const style = getComputedStyle(el);
      return {
        height: el.getBoundingClientRect().height,
        eightLines:
          8 * parseFloat(style.lineHeight) +
          parseFloat(style.paddingTop) +
          parseFloat(style.paddingBottom) +
          parseFloat(style.borderTopWidth) +
          parseFloat(style.borderBottomWidth),
        scrolls: el.scrollHeight > el.clientHeight,
        overflowY: style.overflowY,
      };
    });
    expect(capped.height).toBe(capped.eightLines);
    expect(capped.scrolls).toBe(true);
    expect(capped.overflowY).toBe('auto');

    await composer.fill('');
    expect((await composer.boundingBox())!.height).toBe(oneLine);
  });
});
