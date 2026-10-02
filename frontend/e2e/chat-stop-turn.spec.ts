// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: While a turn runs the composer's send button is a stop button; pressing it asks the server to end the turn (carnet#705)
// ABOUTME: Holds a turn in flight, checks the stop button in both themes, stops it, and reads the stopped notice the turn ends on

import { test, expect, type Page } from '@playwright/test';
import { setupDashboardMocks, loginToDashboard } from './test-helpers';

const CONVERSATION_ID = 'conv-stop';
const QUESTION = 'How does my week look?';
const STOPPED_NOTICE = 'Reply stopped.';

const CONVERSATIONS = {
  conversations: [
    {
      id: CONVERSATION_ID,
      title: 'Half marathon',
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

const EARLIER = [
  { id: 'm1', conversation_id: CONVERSATION_ID, role: 'user', content: 'Hi', created_at: '2026-09-30T15:06:00Z' },
  {
    id: 'm2',
    conversation_id: CONVERSATION_ID,
    role: 'assistant',
    content: 'Hi, ready when you are.',
    created_at: '2026-09-30T15:06:10Z',
  },
];

/** The envelope the server ends a stopped turn with. */
const STOPPED_TURN = {
  turn_id: 'turn-stop',
  user_message: { id: 'm3', role: 'user', content: QUESTION, created_at: '2026-09-30T15:07:00Z' },
  assistant: {
    message: {
      id: 'm4',
      role: 'assistant',
      content: STOPPED_NOTICE,
      finish_reason: 'stopped',
      created_at: '2026-09-30T15:07:05Z',
    },
    blocks: [{ type: 'prose', text: STOPPED_NOTICE }],
    finish_reason: 'stopped',
  },
  conversation_updated_at: '2026-09-30T15:07:05Z',
  telemetry: {
    model: 'stopped',
    provider_name: 'platform',
    tool_calls_count: 0,
    tools_called: [],
    execution_time_ms: 5000,
  },
};

interface Turn {
  /** Stop requests the page sent, by URL. */
  stops: string[];
}

/** Open the thread with a turn endpoint that answers only once it is stopped. */
async function openThread(page: Page): Promise<Turn> {
  const turn: Turn = { stops: [] };
  let release: () => void = () => undefined;
  const stopped = new Promise<void>(resolve => {
    release = resolve;
  });

  await setupDashboardMocks(page, { role: 'user' });
  await page.route('**/api/chat/conversations/*/messages**', async (route, request) => {
    if (request.method() !== 'POST') {
      await route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify({ messages: EARLIER }) });
      return;
    }
    // The turn stays in flight, as a real one does, until the stop lands.
    await stopped;
    await route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify(STOPPED_TURN) });
  });
  await page.route('**/api/chat/conversations/*/stop', async (route, request) => {
    turn.stops.push(new URL(request.url()).pathname);
    await route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify({ stopped: true }) });
    release();
  });
  await page.route('**/api/chat/conversations**', async (route, request) => {
    const url = request.url();
    if (url.includes('/messages') || url.includes('/read') || url.includes('/stop')) {
      await route.fallback();
      return;
    }
    await route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify(CONVERSATIONS) });
  });
  await loginToDashboard(page);
  await page.goto(`/#chat/${CONVERSATION_ID}`);
  await expect(page.getByTestId('message-row').first()).toBeVisible();
  return turn;
}

/** The button's fill and glyph ink, as the browser resolved them. */
async function inks(page: Page) {
  return page.getByTestId('stop-turn-button').evaluate(el => {
    const style = getComputedStyle(el);
    return { fill: style.backgroundColor, glyph: style.color };
  });
}

test.describe('stopping a running turn', () => {
  test('send becomes stop while the turn runs, in both themes, and stop ends the turn', async ({ page }, testInfo) => {
    const turn = await openThread(page);

    const composer = page.getByPlaceholder('Message Dravr...').first();
    await composer.fill(QUESTION);
    const send = page.getByRole('button', { name: 'Send message' });
    const sendBox = (await send.boundingBox())!;
    await send.click();

    const stop = page.getByRole('button', { name: 'Stop this reply' });
    await expect(stop).toBeVisible();
    await expect(stop).toBeEnabled();
    await expect(page.getByRole('button', { name: 'Send message' })).toHaveCount(0);

    // The stop button sits exactly where send sat: the athlete's thumb is
    // already there.
    const stopBox = (await stop.boundingBox())!;
    expect(stopBox.x).toBe(sendBox.x);
    expect(stopBox.width).toBe(sendBox.width);
    expect(stopBox.height).toBe(sendBox.height);

    // Light, then dark: the fill and the glyph must differ in each, or the
    // square is invisible on its own button.
    for (const theme of ['light', 'dark'] as const) {
      await page.evaluate(dark => document.documentElement.classList.toggle('dark', dark), theme === 'dark');
      const { fill, glyph } = await inks(page);
      expect(fill, `${theme}: the stop button is filled`).not.toBe('rgba(0, 0, 0, 0)');
      expect(glyph, `${theme}: the glyph is not the fill`).not.toBe(fill);
      await page.screenshot({ path: testInfo.outputPath(`stop-button-${theme}.png`) });
    }
    await page.evaluate(() => document.documentElement.classList.remove('dark'));

    await stop.click();
    await expect.poll(() => turn.stops).toEqual([`/api/chat/conversations/${CONVERSATION_ID}/stop`]);

    // The turn ends on the stopped notice, behind the question it closed.
    const notice = page.locator('[data-role="assistant"]', { hasText: STOPPED_NOTICE });
    await expect(notice).toBeVisible();
    await expect(page.locator('[data-role="user"]', { hasText: QUESTION })).toBeVisible();
    await expect(page.getByTestId('stop-turn-button')).toHaveCount(0);
    await expect(page.getByRole('button', { name: 'Send message' })).toBeVisible();
    await page.screenshot({ path: testInfo.outputPath('stopped-light.png') });
  });
});
