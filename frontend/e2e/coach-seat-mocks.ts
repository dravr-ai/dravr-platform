// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Mocks for the coach seat: the owner of a coachless room who coaches others but holds no coach access
// ABOUTME: One room, its group, its roster, the coaching platforms and the coach-access request the strip and Group info read

import type { Page } from '@playwright/test';
import type { CoachAccessRequest } from '@pierre/shared-types';
import { loginToDashboard, setupDashboardMocks, ONBOARDED_STATUS } from './test-helpers';

export const SEAT_GROUP_ID = 'group-seat';
export const SEAT_CONVERSATION_ID = 'conv-group-seat';
export const SEAT_GROUP_NAME = 'Léa · Marathon de Montréal';

/** A request the coach filed from this group, waiting on the Dravr team. */
export const PENDING_REQUEST: CoachAccessRequest = {
  id: 'req-seat-1',
  user_id: 'user-123',
  group_id: SEAT_GROUP_ID,
  group_tenant_id: 'tenant-1',
  status: 'pending',
  created_at: '2026-11-12T19:32:00Z',
  decided_at: null,
  decided_by: null,
};

/** The same request, declined by a super-admin the next day. */
export const DECLINED_REQUEST: CoachAccessRequest = {
  ...PENDING_REQUEST,
  status: 'declined',
  decided_at: '2026-11-13T15:00:00Z',
  decided_by: 'admin-1',
};

export interface SeatMockOptions {
  /** The caller's latest coach-access request; `null` when they never asked. */
  request?: CoachAccessRequest | null;
  /** The caller's role in the room's group. */
  role?: 'owner' | 'member';
  /** The onboarding answer "I coach others". */
  coachesOthers?: boolean;
  /** The caller already holds coach access (`manages_roster`). */
  managesRoster?: boolean;
}

/** What the coach did against the mocked server. */
export interface SeatRecord {
  /** Every POST to the coach-access request route, with its body. */
  requests: Array<{ group_id?: string }>;
}

const groupConversation = {
  id: SEAT_CONVERSATION_ID,
  title: SEAT_GROUP_NAME,
  agent_id: 'agent-endurance',
  agent_title: 'Endurance',
  agent_handle: 'endurance',
  group_id: SEAT_GROUP_ID,
  group_name: SEAT_GROUP_NAME,
  message_count: 2,
  unread_count: 0,
  created_at: '2026-11-10T18:42:00Z',
  updated_at: '2026-11-12T19:05:00Z',
  last_message: {
    preview: 'Garde dimanche en endurance facile, 1 h 30 au plus.',
    role: 'assistant',
    created_at: '2026-11-12T19:05:00Z',
  },
};

const groupDetail = {
  id: SEAT_GROUP_ID,
  tenant_id: 'tenant-1',
  name: SEAT_GROUP_NAME,
  description: 'Objectif : Marathon de Montréal, septembre 2027',
  agent_id: 'agent-endurance',
  agent_title: 'Endurance',
  agent_handle: 'endurance',
  coach_user_id: null,
  coach_display_name: null,
  owner_id: 'user-123',
  peer_data_sharing: false,
  respond_mode: 'all',
  digest_mode: 'off',
  max_members: 20,
  is_active: true,
  created_at: '2026-11-10T18:40:00Z',
  updated_at: '2026-11-12T19:05:00Z',
};

function members(role: 'owner' | 'member') {
  return {
    members: [
      {
        id: 'member-1',
        group_id: SEAT_GROUP_ID,
        user_id: 'user-123',
        role,
        peer_sharing_consent: false,
        consent_given_at: null,
        joined_at: '2026-11-10T18:40:00Z',
        display_name: 'Philippe',
      },
      {
        id: 'member-2',
        group_id: SEAT_GROUP_ID,
        user_id: 'user-lea',
        role: role === 'owner' ? 'member' : 'owner',
        peer_sharing_consent: true,
        consent_given_at: '2026-11-10T18:42:00Z',
        joined_at: '2026-11-10T18:42:00Z',
        display_name: 'Léa Gagnon',
      },
    ],
  };
}

/** The room as the transcript serves it: Léa's question and the agent's answer. */
const transcript = {
  entries: [
    {
      id: 'entry-lea-1',
      speaker: 'member',
      withheld: false,
      own: false,
      author_user_id: 'user-lea',
      author_display_name: 'Léa Gagnon',
      content: 'J’ai couru 14 km hier et les jambes étaient lourdes. On garde la sortie longue de dimanche ?',
      message_id: null,
      created_at: '2026-11-12T18:58:00Z',
    },
    {
      id: 'entry-agent-1',
      speaker: 'coach',
      withheld: false,
      own: false,
      author_user_id: 'user-lea',
      author_display_name: 'Léa Gagnon',
      content:
        'Garde dimanche en endurance facile, 1 h 30 au plus. Si tes jambes sont encore lourdes samedi, écris-le ici et on raccourcit.',
      message_id: 'msg-agent-1',
      created_at: '2026-11-12T19:05:00Z',
    },
  ],
  total: 2,
};

/** The two coaching platforms, neither connected yet. */
const coachPlatforms = {
  providers: [
    {
      provider: 'sciotte_trainingpeaks',
      display_name: 'TrainingPeaks',
      description: 'Completed workouts and their training load from TrainingPeaks',
      requires_oauth: false,
      connected: false,
      capabilities: ['activities'],
      consent_required: false,
    },
    {
      provider: 'intervals_icu',
      display_name: 'Intervals.icu',
      description: 'Activities, wellness and your calendar from intervals.icu',
      requires_oauth: false,
      connected: false,
      capabilities: ['activities'],
      consent_required: false,
    },
  ],
};

function json(body: unknown, status = 200) {
  return { status, contentType: 'application/json', body: JSON.stringify(body) };
}

/**
 * Mock the dashboard and the coach's room. A POST to the request route answers
 * with a pending request dated now, and the GET reads it back from then on.
 */
export async function setupCoachSeatMocks(page: Page, options: SeatMockOptions = {}): Promise<SeatRecord> {
  const { role = 'owner', coachesOthers = true, managesRoster = false } = options;
  let latest: CoachAccessRequest | null = options.request ?? null;
  const record: SeatRecord = { requests: [] };

  await setupDashboardMocks(page, { role: 'user', skipProvidersRoute: true, managesRoster });

  await page.route('**/api/providers', (route) => route.fulfill(json(coachPlatforms)));
  // Onboarded, the group step behind them: the room is where they landed.
  await page.route('**/api/me/onboarding-status', (route) =>
    route.fulfill(
      json({
        ...ONBOARDED_STATUS,
        steps: [...ONBOARDED_STATUS.steps, { step_id: 'coach_group', status: 'complete' }],
        coaches_others: coachesOthers,
      }),
    ),
  );
  await page.route('**/api/me/coach-access-request', async (route) => {
    if (route.request().method() === 'POST') {
      record.requests.push((route.request().postDataJSON() as { group_id?: string }) ?? {});
      latest = { ...PENDING_REQUEST, created_at: new Date().toISOString() };
      await route.fulfill(json({ request: latest }, 201));
      return;
    }
    await route.fulfill(json({ request: latest }));
  });

  await page.route('**/api/groups/permissions', (route) =>
    route.fulfill(json({ can_create: true, policy: 'everyone', weekly_digest: false })),
  );
  await page.route(`**/api/groups/${SEAT_GROUP_ID}`, (route) => route.fulfill(json(groupDetail)));
  await page.route(`**/api/groups/${SEAT_GROUP_ID}/members`, (route) => route.fulfill(json(members(role))));
  await page.route(`**/api/groups/${SEAT_GROUP_ID}/invites`, (route) => route.fulfill(json({ invites: [] })));
  await page.route(`**/api/groups/${SEAT_GROUP_ID}/stats`, (route) =>
    route.fulfill(
      json({
        stats: {
          total_members: 2,
          active_members: 1,
          avg_weekly_volume_km: 32,
          avg_ctl: null,
          flagged_members: 0,
          weekly_trend: 'stable',
        },
        scope: { visible_members: 1, roster_members: 2 },
      }),
    ),
  );
  await page.route(`**/api/groups/${SEAT_GROUP_ID}/health`, (route) =>
    route.fulfill(json({ flags: [], total: 0, scope: { visible_members: 1, roster_members: 2 } })),
  );
  await page.route(`**/api/groups/${SEAT_GROUP_ID}/members/me/coach-sharing`, (route) =>
    route.fulfill(json({ consent: true })),
  );
  await page.route(`**/api/chat/groups/${SEAT_GROUP_ID}/transcript*`, (route) => route.fulfill(json(transcript)));
  await page.route('**/api/agents**', (route) =>
    route.fulfill(
      json({ agents: [], total: 0, metadata: { timestamp: '2026-11-12T00:00:00Z', api_version: 'v1' } }),
    ),
  );
  await page.route('**/api/commands**', (route) => route.fulfill(json({ commands: [] })));
  await page.route(`**/api/chat/conversations/${SEAT_CONVERSATION_ID}/messages`, (route) =>
    route.fulfill(json({ messages: [] })),
  );
  await page.route(`**/api/chat/conversations/${SEAT_CONVERSATION_ID}/verdicts`, (route) =>
    route.fulfill(json({ verdicts: [], total: 0 })),
  );
  await page.route(`**/api/chat/conversations/${SEAT_CONVERSATION_ID}/participants`, (route) =>
    route.fulfill(json({ participants: [] })),
  );
  await page.route(/\/api\/chat\/conversations(\?.*)?$/, (route) =>
    route.fulfill(json({ conversations: [groupConversation], total: 1, limit: 50, offset: 0 })),
  );

  return record;
}

/** Sign in and open the coach's room straight from its link. */
export async function openSeatRoom(page: Page) {
  await loginToDashboard(page);
  await page.evaluate((id) => {
    window.location.hash = `#chat/${id}`;
  }, SEAT_CONVERSATION_ID);
  await page.getByTestId('thread-header').waitFor({ timeout: 15000 });
}
