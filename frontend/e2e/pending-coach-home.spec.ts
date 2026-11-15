// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: E2E for the pending coach Home (carnet#1529): « Préparer ton coaching » in place of the athlete Home, and the grant that ends it
// ABOUTME: Also the switch only a coach who trains has, and no provider nudge in a room for a coach who does not train

import { test, expect, type Page } from '@playwright/test';
import { PENDING_REQUEST, SEAT_CONVERSATION_ID, setupCoachSeatMocks } from './coach-seat-mocks';
import { loginToDashboard } from './test-helpers';

// The human coach's own access and Home, in English.
const SCOPE = 'Your coach Home opens as soon as your coach access is on'; // humanCoach.pendingHome.scope
const SCOPE_PENDING = "Coach access pending · your coach Home opens as soon as it's on"; // humanCoach.pendingHome.scopePending

/** Sign in and land on Home, where a coach without access is taken. */
async function openPendingHome(page: Page) {
  await loginToDashboard(page);
  await expect(page.getByTestId('pending-coach-home')).toBeVisible({ timeout: 15000 });
}

test.describe('Pending coach Home — the checklist', () => {
  test('a coach who does not train lands on « Préparer ton coaching », never the athlete Home', async ({ page }) => {
    const record = await setupCoachSeatMocks(page, { trains: false });
    await openPendingHome(page);

    await expect(page.getByRole('heading', { level: 1, name: 'My athletes' })).toBeVisible();
    await expect(page.getByTestId('coach-home-switch')).toHaveCount(0);
    await expect(page.getByTestId('pending-coach-scope')).toHaveText(SCOPE);
    await expect(page.getByTestId('coach-setup-count')).toHaveText('0 of 3');
    await expect(page.getByTestId('coach-setup-row-seat')).toHaveAttribute('data-state', 'current');
    // No athlete nudge anywhere on the page.
    await expect(page.getByTestId('home-getting-started')).toHaveCount(0);
    await expect(page.getByTestId('connect-provider-banner')).toHaveCount(0);

    const seat = page.getByTestId('coach-setup-row-seat');
    await seat.getByRole('button', { name: 'Request coach access' }).click();
    await expect(seat.getByTestId('coach-seat-row')).toHaveAttribute('data-state', 'pending');
    await expect(seat.getByTestId('coach-access-pending')).toHaveText(
      /^Request sent .+\. We'll let you know here and by email\.$/,
    );
    await expect(page.getByTestId('pending-coach-scope')).toHaveText(SCOPE_PENDING);
    expect(record.requests).toEqual([{}]);

    // The room's group is listed, where the agent already answers.
    await expect(page.getByTestId('coach-setup-room-group-seat')).toBeVisible();
    // TrainingPeaks and Intervals.icu read the same (D3).
    const platforms = page.getByTestId('coach-setup-platforms');
    await expect(platforms.getByText('TrainingPeaks')).toBeVisible();
    await expect(platforms.getByText('Intervals.icu')).toBeVisible();
  });

  test('a grant turns Home into the coach Home without signing in again', async ({ page }) => {
    const record = await setupCoachSeatMocks(page, { request: PENDING_REQUEST, trains: false });
    await openPendingHome(page);

    await record.grant();
    await page.evaluate(() => {
      window.dispatchEvent(new Event('visibilitychange'));
      window.dispatchEvent(new Event('focus'));
    });

    await expect(page.getByTestId('coach-home')).toBeVisible({ timeout: 15000 });
    await expect(page.getByTestId('pending-coach-home')).toHaveCount(0);
    // A coach who does not train has no « Mon entraînement » there either.
    await expect(page.getByTestId('coach-home-title')).toHaveText('My athletes');
    await expect(page.getByTestId('coach-home-switch')).toHaveCount(0);
  });

  test('a coach who trains keeps their own Home behind the switch', async ({ page }) => {
    await setupCoachSeatMocks(page, { trains: true });
    await openPendingHome(page);

    await page.getByTestId('coach-home-switch-me').click();
    await expect(page.getByTestId('coach-home-personal')).toBeVisible();
    await expect(page).toHaveURL(/#home\/me$/);
    await page.getByTestId('coach-home-switch-athletes').click();
    await expect(page.getByTestId('pending-coach-home')).toBeVisible();
  });
});

test.describe('Pending coach Home — no provider nudge for a coach who does not train', () => {
  async function openRoom(page: Page) {
    await loginToDashboard(page);
    await page.evaluate((id) => {
      window.location.hash = `#chat/${id}`;
    }, SEAT_CONVERSATION_ID);
    await page.getByTestId('thread-header').waitFor({ timeout: 15000 });
  }

  test('a coached room asks a coach who does not train to connect nothing', async ({ page }) => {
    await setupCoachSeatMocks(page, { managesRoster: true, trains: false });
    await openRoom(page);
    await expect(page.getByTestId('thread-header')).toBeVisible();
    await page.waitForTimeout(500);
    await expect(page.getByTestId('connect-provider-banner')).toHaveCount(0);
  });

  test('the same room still asks a coach who trains, with nothing connected', async ({ page }) => {
    await setupCoachSeatMocks(page, { managesRoster: true, trains: true });
    await openRoom(page);
    await expect(page.getByTestId('connect-provider-banner')).toBeVisible();
  });
});
