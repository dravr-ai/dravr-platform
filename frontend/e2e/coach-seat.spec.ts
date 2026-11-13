// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: E2E for the coach seat (carnet#1520): the room strip, the header state, and Group info's first block
// ABOUTME: An owner-coach without access finds the filled request without opening anything; asking dates it

import { test, expect } from '@playwright/test';
import {
  DECLINED_REQUEST,
  SEAT_GROUP_ID,
  openSeatRoom,
  setupCoachSeatMocks,
} from './coach-seat-mocks';
import { useFrench } from './text-collision';

test.describe('Coach seat — the room', () => {
  test('an owner-coach without access sees the filled request at the top of the room', async ({ page }) => {
    await setupCoachSeatMocks(page);
    await openSeatRoom(page);

    const strip = page.getByTestId('coach-seat-strip');
    await expect(strip).toBeVisible();
    await expect(strip).toHaveAttribute('data-state', 'to_request');
    await expect(strip.getByRole('heading', { name: "You're not this group's coach yet" })).toBeVisible();
    await expect(strip.getByRole('button', { name: 'Request coach access' })).toBeVisible();
    // The seat strip takes the provider nudge's place.
    await expect(page.getByTestId('connect-provider-banner')).toHaveCount(0);
    await expect(page.getByTestId('thread-status')).toHaveText('coach access to request');
    // Nothing is pending before a request exists.
    await expect(strip).not.toContainText(/pending/i);
    await expect(page.getByTestId('thread-header')).not.toContainText(/pending/i);
  });

  test('asking dates the request in the strip, the header and Group info', async ({ page }) => {
    const record = await setupCoachSeatMocks(page);
    await openSeatRoom(page);

    await page.getByTestId('coach-seat-strip').getByRole('button', { name: 'Request coach access' }).click();

    const strip = page.getByTestId('coach-seat-strip');
    await expect(strip).toHaveAttribute('data-state', 'pending');
    await expect(strip.getByRole('heading')).toHaveText(/^Coach access requested on /);
    await expect(strip).toContainText('usually within 24 hours');
    await expect(strip.getByRole('button', { name: 'Request coach access' })).toHaveCount(0);
    await expect(page.getByTestId('thread-status')).toHaveText('coach access pending');
    expect(record.requests).toEqual([{ group_id: SEAT_GROUP_ID }]);

    // The header opens Group info, whose first block is the dated timeline.
    await page.getByTestId('conversation-header-title').click();
    const block = page.getByTestId('coach-seat-block');
    await expect(block).toHaveAttribute('data-state', 'pending');
    await expect(block.getByRole('list', { name: 'Where your request stands' })).toBeVisible();
    await expect(block.getByTestId('coach-seat-step-sent')).toContainText('Request sent');
    await expect(block.getByTestId('coach-seat-step-review')).toHaveAttribute('aria-current', 'step');
    await expect(block).not.toContainText(/email/i);
  });

  test('a member of the room who does not own it sees no seat', async ({ page }) => {
    await setupCoachSeatMocks(page, { role: 'member' });
    await openSeatRoom(page);

    await expect(page.getByTestId('thread-title')).toBeVisible();
    await expect(page.getByTestId('coach-seat-strip')).toHaveCount(0);
    await expect(page.getByTestId('thread-status')).toHaveCount(0);
  });
});

test.describe('Coach seat — Group info', () => {
  test('leads the drawer with the block, its one filled button and the two coaching platforms as plain rows', async ({ page }) => {
    await setupCoachSeatMocks(page);
    await openSeatRoom(page);
    await page.getByTestId('coach-seat-strip').getByRole('button', { name: 'Link a coach account' }).click();

    const panel = page.getByTestId('group-info-panel');
    const block = panel.getByTestId('coach-seat-block');
    await expect(block).toBeVisible();
    await expect(block).toHaveAttribute('data-state', 'to_request');
    await expect(block.getByRole('button', { name: 'Request coach access' })).toBeVisible();
    await expect(panel.locator('.btn-primary')).toHaveCount(1);
    const routes = block.getByTestId('coach-seat-routes');
    await expect(routes.getByTestId('provider-card-sciotte_trainingpeaks')).toHaveText('TrainingPeaksConnect');
    await expect(routes.getByTestId('provider-card-intervals_icu')).toHaveText('Intervals.icuConnect');
    // Nothing tells the owner about to coach that the group has none.
    await expect(panel).not.toContainText('No human coach');
    await expect(panel).not.toContainText('Create a Coach invite');
    await expect(panel.getByTestId('coach-sharing-row')).toHaveCount(0);
    await expect(panel.getByTestId('group-info-coach-seat')).toHaveText('You · not the coach yet');
  });

  test('a declined request is dated and offers to ask again', async ({ page }) => {
    const record = await setupCoachSeatMocks(page, { request: DECLINED_REQUEST });
    await openSeatRoom(page);
    await page.getByTestId('conversation-header-title').click();

    const block = page.getByTestId('coach-seat-block');
    await expect(block).toHaveAttribute('data-state', 'declined');
    await expect(block.getByTestId('coach-access-declined')).toContainText('You can ask again.');
    await block.getByRole('button', { name: 'Ask again' }).click();
    await expect(block).toHaveAttribute('data-state', 'pending');
    expect(record.requests).toEqual([{ group_id: SEAT_GROUP_ID }]);
  });
});

test.describe('Coach seat — phone, in French', () => {
  test.use({ viewport: { width: 390, height: 844 } });

  test('the request is in the room without opening anything, and first in Group info', async ({ page }) => {
    await useFrench(page);
    await setupCoachSeatMocks(page);
    await openSeatRoom(page);

    const strip = page.getByTestId('coach-seat-strip');
    await expect(strip.getByRole('heading', { name: "Tu n'es pas encore le coach de ce groupe" })).toBeInViewport();
    const ask = strip.getByRole('button', { name: "Demander l'accès coach" });
    await expect(ask).toBeInViewport();
    // A full-width, 44px touch target on a phone.
    const box = await ask.boundingBox();
    expect(box?.height ?? 0).toBeGreaterThanOrEqual(44);
    await expect(page.getByTestId('thread-status')).toHaveText('accès coach à demander');

    await page.getByTestId('conversation-header-title').click();
    const block = page.getByTestId('coach-seat-block');
    await expect(block.getByRole('button', { name: "Demander l'accès coach" })).toBeInViewport();
  });
});
