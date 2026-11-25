// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The mocked server a coach's Home runs against — the roster dated from today, "Traité" and its undo, the morning brief, the replies and one group room
// ABOUTME: Shared by the Coach Home spec, the design sweep and the accessibility scan, so all three read the same roster

import type { Page } from '@playwright/test';
import type { CoachBrief, CoachBriefUpdate, CoachRosterResponse } from '@pierre/shared-types';
import { applyBriefUpdate } from '@pierre/ui-logic';
import { setupDashboardMocks, loginToDashboard } from './test-helpers';
import { buildBrief, buildRoster } from '../src/test/coachRosterFixtures';

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
  /** Bodies of `PUT /api/coach/brief`. */
  brief: CoachBriefUpdate[];
  /** Bodies of `POST /api/coach/brief/opened`. */
  opened: unknown[];
}

/** Mocks the coach's roster, "Traité" and its undo, the morning brief, the replies, and the one group room. */
export async function mockCoach(
  page: Page,
  options: { fillers?: number; language?: string; roster?: (roster: CoachRosterResponse) => CoachRosterResponse } = {},
): Promise<Posted> {
  const posted: Posted = { dismiss: [], restore: [], brief: [], opened: [] };
  await setupDashboardMocks(page, { role: 'user', displayName: 'Phil', managesRoster: true, language: options.language });
  const built = buildRoster(localToday(), options.fillers ?? 0);
  const roster = options.roster ? options.roster(built) : built;
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
  // The morning brief: read, changed (the answer is the brief with the change
  // applied, lists replaced whole), and told of each athlete opened.
  let brief: CoachBrief = buildBrief(localToday());
  await page.route('**/api/coach/brief', async (route, request) => {
    if (request.method() === 'PUT') {
      const body = request.postDataJSON() as CoachBriefUpdate;
      posted.brief.push(body);
      brief = applyBriefUpdate(brief, body);
    }
    await route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify(brief) });
  });
  await page.route('**/api/coach/brief/opened', async (route) => {
    posted.opened.push(route.request().postDataJSON());
    await route.fulfill({ status: 204, body: '' });
  });
  await page.route('**/api/coach/replies**', (route) =>
    route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify({ groups: [], athletes: [], replies: [], next_before: null }) }),
  );
  // "Write in the group": the server finds or opens the caller's room.
  await page.route('**/api/chat/groups/*/room', (route) =>
    route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify(GROUP_ROOM) }),
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
