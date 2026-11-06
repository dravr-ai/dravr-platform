// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Measures the chat reply's action row on a phone: every target 44x44, nothing overlapping, the row wrapping
// ABOUTME: Also pins the desktop size, since the `touch-target` rule must only apply under `lg` or on a coarse pointer

import { test, expect, type Locator, type Page } from '@playwright/test';
import { setupDashboardMocks, loginToDashboard, openPersonalConversation } from '../test-helpers';

const PHONE = { width: 393, height: 852 };
/** The narrowest phone still sold: too narrow for the five 44px targets on one line. */
const NARROW_PHONE = { width: 320, height: 568 };
const DESKTOP = { width: 1280, height: 900 };
const MIN_TARGET_PX = 44;
/** `p-0.5` around the 14px icon: the size the desktop row has always drawn. */
const DESKTOP_TARGET_PX = 18;
const ICON_PX = 14;

const CONVERSATION_ID = 'conv-reply-actions';
const CONVERSATION_TITLE = 'Sunday long run';
const FIRST_REPLY = 'That fade at 26k is the interesting part.';
const SECOND_REPLY = 'How was fuelling after the second hour?';

function assistantRow(id: string, content: string, minute: string) {
  return {
    id,
    conversation_id: CONVERSATION_ID,
    role: 'assistant',
    content,
    created_at: `2026-08-20T10:${minute}:00Z`,
    finish_reason: 'stop',
    // Regenerate shows only on a turn that changed nothing, so this draws the
    // fullest row an athlete can get: copy, share, both ratings and regenerate.
    turn_attempted_write: false,
  };
}

async function openThread(page: Page) {
  await setupDashboardMocks(page, { role: 'user' });
  await page.route(/\/api\/chat\/conversations(\?.*)?$/, async (route) =>
    route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({
        conversations: [
          {
            id: CONVERSATION_ID,
            title: CONVERSATION_TITLE,
            agent_id: null,
            created_at: '2026-08-20T10:00:00Z',
            updated_at: '2026-08-20T10:02:00Z',
            message_count: 3,
            unread_count: 0,
          },
        ],
        total: 1,
        limit: 50,
        offset: 0,
      }),
    }),
  );
  await page.route(`**/api/chat/conversations/${CONVERSATION_ID}/messages`, async (route, request) => {
    if (request.method() !== 'GET') {
      await route.fallback();
      return;
    }
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({
        messages: [
          {
            id: 'm-user',
            conversation_id: CONVERSATION_ID,
            role: 'user',
            content: 'Did 32k this morning, felt strong until 26.',
            created_at: '2026-08-20T10:00:00Z',
          },
          assistantRow('m-first', FIRST_REPLY, '01'),
          // The same author a minute later: a continuation row, whose time
          // joins the action row and takes width from it.
          assistantRow('m-second', SECOND_REPLY, '02'),
        ],
      }),
    });
  });
  await page.route(`**/api/chat/conversations/${CONVERSATION_ID}/verdicts`, async (route) =>
    route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({ verdicts: [], total: 0 }),
    }),
  );

  await loginToDashboard(page);
  await openPersonalConversation(page, CONVERSATION_TITLE);
  await expect(page.getByText(SECOND_REPLY)).toBeVisible({ timeout: 10000 });
}

function replyRow(page: Page, reply: string): Locator {
  return page.locator('[data-testid="message-row"][data-role="assistant"]').filter({ hasText: reply });
}

function replyButtons(row: Locator): Locator {
  return row.getByRole('group', { name: 'Message actions' }).getByRole('button');
}

async function boxOf(locator: Locator) {
  const box = await locator.boundingBox();
  if (box === null) throw new Error('the element has no layout box');
  return box;
}

type Box = Awaited<ReturnType<typeof boxOf>>;

function overlaps(a: Box, b: Box): boolean {
  return a.x < b.x + b.width && b.x < a.x + a.width && a.y < b.y + b.height && b.y < a.y + a.height;
}

/** Every box in the row, pairwise apart and inside the column that holds them. */
async function expectApartAndInsideColumn(row: Locator, parts: Locator[]) {
  const column = await boxOf(row.getByTestId('message-column'));
  const boxes = await Promise.all(parts.map(boxOf));
  boxes.forEach((box, i) => {
    expect(box.x, `part ${i} starts inside the column`).toBeGreaterThanOrEqual(column.x);
    expect(box.x + box.width, `part ${i} ends inside the column`).toBeLessThanOrEqual(column.x + column.width);
    boxes.slice(i + 1).forEach((other, j) => {
      expect(overlaps(box, other), `part ${i} overlaps part ${i + 1 + j}`).toBe(false);
    });
  });
}

async function expectNoSidewaysScroll(page: Page) {
  const { scrollWidth, clientWidth } = await page.evaluate(() => ({
    scrollWidth: document.documentElement.scrollWidth,
    clientWidth: document.documentElement.clientWidth,
  }));
  expect(scrollWidth).toBeLessThanOrEqual(clientWidth);
}

test.describe('The reply action row on a 393px phone', () => {
  test.use({ viewport: PHONE });

  test('every action is a 44x44 target around an icon that stayed its size', async ({ page }) => {
    await openThread(page);
    const buttons = replyButtons(replyRow(page, FIRST_REPLY));
    // Copy, share, both ratings, regenerate: a smaller row would measure less.
    await expect(buttons).toHaveCount(5);

    for (const button of await buttons.all()) {
      const box = await boxOf(button);
      const name = await button.getAttribute('title');
      expect(box.width, `${name} width`).toBeGreaterThanOrEqual(MIN_TARGET_PX);
      expect(box.height, `${name} height`).toBeGreaterThanOrEqual(MIN_TARGET_PX);

      const icon = await boxOf(button.locator('svg'));
      expect(icon.width, `${name} icon width`).toBe(ICON_PX);
      expect(icon.height, `${name} icon height`).toBe(ICON_PX);
      // Centred: the growth has to surround the icon, or the row reads as misaligned.
      expect(Math.abs(icon.x + icon.width / 2 - (box.x + box.width / 2)), `${name} centred across`).toBeLessThan(1);
      expect(Math.abs(icon.y + icon.height / 2 - (box.y + box.height / 2)), `${name} centred down`).toBeLessThan(1);
    }
  });

  test('a row whose time takes width keeps everything apart and inside the column', async ({ page }) => {
    await openThread(page);
    const row = replyRow(page, SECOND_REPLY);
    const buttons = await replyButtons(row).all();

    await expectApartAndInsideColumn(row, [row.getByTestId('message-time'), ...buttons]);
    await expectNoSidewaysScroll(page);
  });
});

test.describe('The reply action row on a 320px phone', () => {
  test.use({ viewport: NARROW_PHONE });

  test('the buttons wrap onto a second line rather than leave the column', async ({ page }) => {
    await openThread(page);
    const row = replyRow(page, FIRST_REPLY);
    const buttons = await replyButtons(row).all();

    await expectApartAndInsideColumn(row, buttons);
    await expectNoSidewaysScroll(page);
    const first = await boxOf(buttons[0]);
    const last = await boxOf(buttons[buttons.length - 1]);
    expect(last.y, 'the last button took the next line').toBeGreaterThanOrEqual(first.y + first.height);
    expect(first.width).toBeGreaterThanOrEqual(MIN_TARGET_PX);
    expect(first.height).toBeGreaterThanOrEqual(MIN_TARGET_PX);
  });

  test('a row whose time takes width gives the time its own line', async ({ page }) => {
    await openThread(page);
    const row = replyRow(page, SECOND_REPLY);
    const time = row.getByTestId('message-time');
    const buttons = await replyButtons(row).all();

    await expectApartAndInsideColumn(row, [time, ...buttons]);
    await expectNoSidewaysScroll(page);
    const timeBox = await boxOf(time);
    const first = await boxOf(buttons[0]);
    expect(first.y, 'the buttons sit under the time').toBeGreaterThanOrEqual(timeBox.y + timeBox.height);
  });
});

test.describe('The reply action row on a fine pointer at desktop width', () => {
  test.use({ viewport: DESKTOP });

  test('keeps the compact row the phone rule must not reach', async ({ page }) => {
    await openThread(page);
    const buttons = replyButtons(replyRow(page, FIRST_REPLY));
    await expect(buttons).toHaveCount(5);

    const tops = new Set<number>();
    for (const button of await buttons.all()) {
      const box = await boxOf(button);
      expect(box.width).toBe(DESKTOP_TARGET_PX);
      expect(box.height).toBe(DESKTOP_TARGET_PX);
      tops.add(box.y);
    }
    expect(tops.size, 'all five sit on one line').toBe(1);
  });
});

test.describe('the other controls beside a reply, on a phone', () => {
  test.use({ viewport: PHONE });

  test('the reason form after a poor rating is 44px tall to type in and to send', async ({ page }) => {
    await openThread(page);
    await page.route('**/api/chat/**/feedback', async (route) =>
      route.fulfill({ status: 200, contentType: 'application/json', body: '{}' }),
    );
    await replyRow(page, SECOND_REPLY).hover();
    await replyRow(page, SECOND_REPLY).getByTitle('Poor response').click();

    for (const control of [
      page.getByPlaceholder('What went wrong? (optional)'),
      page.getByRole('button', { name: 'Send', exact: true }),
    ]) {
      const box = await boxOf(control);
      expect(box.height, 'reason control height').toBeGreaterThanOrEqual(MIN_TARGET_PX);
      expect(box.width, 'reason control width').toBeGreaterThanOrEqual(MIN_TARGET_PX);
    }
    await expectNoSidewaysScroll(page);
  });
});
