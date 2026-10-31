// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The mocked server a coach's Home runs against — the roster dated from today, "Traité" and its undo, the replies and one group room
// ABOUTME: Shared by the Coach Home spec, the design sweep and the accessibility scan, so all three read the same roster

import type { Page } from '@playwright/test';
import { setupDashboardMocks, loginToDashboard } from './test-helpers';
import { buildRoster } from '../src/test/coachRosterFixtures';

export function localToday(): string {
  const now = new Date();
  const month = String(now.getMonth() + 1).padStart(2, '0');
  const day = String(now.getDate()).padStart(2, '0');
  return `${now.getFullYear()}-${month}-${day}`;
}

const GROUP_ROOM = {
  id: 'room-tri',
  title: 'Tri — longue distance',
  agent_id: 'agent-endurance',
  group_id: 'g-tri',
  group_name: 'Tri — longue distance',
  message_count: 2,
  unread_count: 0,
  created_at: '2026-10-01T10:00:00Z',
  updated_at: '2026-10-29T15:00:00Z',
  last_message: { preview: 'Bonne semaine.', role: 'assistant', created_at: '2026-10-29T15:00:00Z' },
};

export interface Posted {
  dismiss: unknown[];
  restore: unknown[];
}

/** Mocks the coach's roster, "Traité" and its undo, the replies, and the one group room. */
export async function mockCoach(page: Page, options: { fillers?: number } = {}): Promise<Posted> {
  const posted: Posted = { dismiss: [], restore: [] };
  await setupDashboardMocks(page, { role: 'user', displayName: 'Phil', managesRoster: true });
  const roster = buildRoster(localToday(), options.fillers ?? 0);
  await page.route('**/api/coach/roster**', (route) =>
    route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify(roster) }),
  );
  await page.route('**/api/coach/attention/dismiss', async (route) => {
    const body = route.request().postDataJSON() as { reason_id: string; fingerprint: string };
    posted.dismiss.push(body);
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({ reason_id: body.reason_id, fingerprint: body.fingerprint, dismissed_at: new Date().toISOString() }),
    });
  });
  await page.route('**/api/coach/attention/restore', async (route) => {
    const body = route.request().postDataJSON() as { reason_id: string };
    posted.restore.push(body);
    await route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify({ reason_id: body.reason_id, restored: true }) });
  });
  await page.route('**/api/coach/replies**', (route) =>
    route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify({ groups: [], athletes: [], replies: [], next_before: null }) }),
  );
  // The list pattern also matches the messages and read-marker URLs; those fall through.
  await page.route('**/api/chat/conversations**', async (route, request) => {
    const url = request.url();
    if (url.includes('/messages') || url.includes('/read')) {
      await route.fallback();
      return;
    }
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({ conversations: [GROUP_ROOM], total: 1, limit: 50, offset: 0 }),
    });
  });
  await loginToDashboard(page);
  return posted;
}
