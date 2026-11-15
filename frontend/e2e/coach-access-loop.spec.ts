// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: E2E for the coach-access loop (carnet#1521, #1522): withdraw and ask again, a grant landing without signing in again
// ABOUTME: Also the decline reason, the request with no group in Settings, and an owner who never said they coach others

import { test, expect, type Page } from '@playwright/test';
import { DECLINED_REQUEST, PENDING_REQUEST, SEAT_GROUP_ID, openSeatRoom, setupCoachSeatMocks } from './coach-seat-mocks';
import { loginToDashboard } from './test-helpers';

/** The signed-in user the app keeps, as the shell stored it. */
async function storedManagesRoster(page: Page): Promise<boolean | undefined> {
  return page.evaluate(() => {
    for (let i = 0; i < localStorage.length; i += 1) {
      const raw = localStorage.getItem(localStorage.key(i) ?? '') ?? '';
      if (raw.includes('"manages_roster"')) {
        try {
          return (JSON.parse(raw) as { manages_roster?: boolean }).manages_roster;
        } catch {
          // Not the user record.
        }
      }
    }
    return undefined;
  });
}

test.describe('Coach access — withdraw and ask again', () => {
  test('a waiting request is withdrawn from the room and asked again at once', async ({ page }) => {
    const record = await setupCoachSeatMocks(page, { request: PENDING_REQUEST });
    await openSeatRoom(page);

    const strip = page.getByTestId('coach-seat-strip');
    await expect(strip).toHaveAttribute('data-state', 'pending');
    await strip.getByRole('button', { name: 'Cancel request' }).click();

    await expect(strip).toHaveAttribute('data-state', 'to_request');
    await expect(strip.getByTestId('coach-access-cancelled')).toHaveText(
      'Request cancelled. You can ask again whenever you like.',
    );
    expect(record.cancels).toBe(1);
    await expect(page.getByTestId('thread-status')).toHaveText('coach access to request');

    await strip.getByRole('button', { name: 'Request coach access' }).click();
    await expect(strip).toHaveAttribute('data-state', 'pending');
    expect(record.requests).toEqual([{ group_id: SEAT_GROUP_ID }]);
  });
});

test.describe('Coach access — the grant lands', () => {
  test('the list row flips and the session picks up coach access without signing in again', async ({ page }) => {
    const record = await setupCoachSeatMocks(page, { request: PENDING_REQUEST });
    await openSeatRoom(page);

    // While it waits, the room's row says so in place of its last words.
    await expect(page.getByTestId('conversation-seat-line')).toHaveText(/^Coach access pending · requested /);
    expect(await storedManagesRoster(page)).toBe(false);

    // A super-admin grants it; the app reads the request again on focus.
    await record.grant();
    await page.evaluate(() => {
      window.dispatchEvent(new Event('visibilitychange'));
      window.dispatchEvent(new Event('focus'));
    });

    await expect.poll(() => storedManagesRoster(page), { timeout: 15000 }).toBe(true);
    await expect(page.getByTestId('conversation-seat-line')).toHaveText(/^You're the coach · since \d{2}:\d{2}$/);
    // The room has its coach now: the request is no longer offered there.
    await expect(page.getByTestId('coach-seat-strip')).toHaveCount(0);
  });
});

test.describe('Coach access — the decline reason', () => {
  test('Group info shows the reason the team gave beside Ask again', async ({ page }) => {
    await setupCoachSeatMocks(page, {
      request: { ...DECLINED_REQUEST, decline_reason: 'We could not verify a coaching account.' },
    });
    await openSeatRoom(page);
    await page.getByTestId('conversation-header-title').click();

    const block = page.getByTestId('coach-seat-block');
    await expect(block).toHaveAttribute('data-state', 'declined');
    await expect(block.getByTestId('coach-access-decline-reason')).toHaveText(
      'Reason: We could not verify a coaching account.',
    );
    await expect(block.getByRole('button', { name: 'Ask again' })).toBeVisible();
  });

  test('asking again replaces the decline: withdrawing that ask shows the plain ask', async ({ page }) => {
    const record = await setupCoachSeatMocks(page, {
      request: { ...DECLINED_REQUEST, decline_reason: 'We could not verify a coaching account.' },
    });
    await openSeatRoom(page);

    const strip = page.getByTestId('coach-seat-strip');
    await strip.getByRole('button', { name: 'Ask again' }).click();
    await expect(strip).toHaveAttribute('data-state', 'pending');
    await strip.getByRole('button', { name: 'Cancel request' }).click();

    await expect(strip).toHaveAttribute('data-state', 'to_request');
    await expect(strip.getByRole('button', { name: 'Request coach access' })).toBeVisible();
    await expect(strip.getByTestId('coach-access-decline-reason')).toHaveCount(0);
    expect(record.cancels).toBe(1);
    // That the server's own read skips the replaced decline after a reload is
    // pinned by coach_access_request_test (the mock answers as it does).
  });
});

test.describe('Coach access — who is offered the request', () => {
  test('an owner who never said they coach others is offered it on their coachless room', async ({ page }) => {
    await setupCoachSeatMocks(page, { coachesOthers: false });
    await openSeatRoom(page);

    await expect(page.getByTestId('coach-seat-strip')).toHaveAttribute('data-state', 'to_request');
  });

  test('a coach with no group asks from Settings, then withdraws', async ({ page }) => {
    const record = await setupCoachSeatMocks(page);
    await loginToDashboard(page);
    await page.evaluate(() => {
      window.location.hash = '#settings/coaching';
    });

    const panel = page.getByTestId('settings-coach-access').getByTestId('coach-seat-panel');
    await expect(panel).toHaveAttribute('data-state', 'to_request');
    await panel.getByRole('button', { name: 'Request coach access' }).click();
    await expect(panel).toHaveAttribute('data-state', 'pending');
    expect(record.requests).toEqual([{}]);

    await panel.getByRole('button', { name: 'Cancel request' }).click();
    await expect(panel).toHaveAttribute('data-state', 'to_request');
    expect(record.cancels).toBe(1);
  });
});
