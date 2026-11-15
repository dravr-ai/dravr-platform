// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Mocks for the coach seat: the owner of a coachless room who holds no coach access, and what a decision does to it
// ABOUTME: One room, its group, its roster, the coaching platforms, the request, the pending coach Home's checklist, a grant, or a coached room

import type { Page } from '@playwright/test';
import type { CoachAccessRequest, CoachSetupResponse } from '@pierre/shared-types';
import { localToday } from '@pierre/ui-logic';
import { loginToDashboard, setupDashboardMocks, ONBOARDED_STATUS } from './test-helpers';
import { buildRoster } from '../src/test/coachRosterFixtures';

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

/** The same request, declined by an admin the next day. */
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
  /**
   * Who coaches the room: nobody (the default, the seat still to win), the
   * caller (`'viewer'`, as a grant leaves it), or another person
   * (`'someone-else'`, an athlete's room with their coach in it).
   */
  coach?: 'viewer' | 'someone-else';
  /**
   * The coach trains too (« J'entraîne et je m'entraîne »); `false` records
   * the athlete steps waived, as the coach-only answer does (carnet#1529).
   */
  trains?: boolean;
  /** What the pending coach Home's checklist holds beyond the room's group (carnet#1529). */
  setup?: Partial<CoachSetupResponse>;
}

/** What the coach did against the mocked server, and what the team can do to it. */
export interface SeatRecord {
  /** Every POST to the coach-access request route, with its body. */
  requests: Array<{ group_id?: string }>;
  /** How many times the coach withdrew the waiting request (DELETE). */
  cancels: number;
  /**
   * An admin grants the request now: it reads granted, the room has the
   * coach attached and the grant's line as its last words, and the session
   * reads coach access — as the server leaves them after a grant.
   */
  grant: () => Promise<void>;
}

/** The grant's line in the coach's copy of the room, as the server writes it. */
export const GRANT_ROOM_LINE =
  'Philippe is now this group’s coach. Your athletes see this message and choose what they share with you.';

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
  const { role = 'owner', coachesOthers = true, managesRoster = false, trains = true } = options;
  let latest: CoachAccessRequest | null = options.request ?? null;
  let granted = managesRoster;
  // A member who does not own the room: the group names someone else as its owner.
  let group: Record<string, unknown> = {
    ...groupDetail,
    owner_id: role === 'owner' ? 'user-123' : 'user-lea',
    ...(options.coach === 'viewer' ? { coach_user_id: 'user-123', coach_display_name: 'Philippe' } : {}),
    ...(options.coach === 'someone-else' ? { coach_user_id: 'user-marc', coach_display_name: 'Marc Tremblay' } : {}),
  };
  let conversation: Record<string, unknown> = { ...groupConversation };
  const record: SeatRecord = {
    requests: [],
    cancels: 0,
    grant: async () => {
      const decidedAt = new Date().toISOString();
      granted = true;
      latest = { ...(latest ?? PENDING_REQUEST), status: 'granted', decided_at: decidedAt, decided_by: 'admin-1' };
      group = { ...group, coach_user_id: 'user-123', coach_display_name: 'Philippe' };
      conversation = {
        ...conversation,
        unread_count: 1,
        updated_at: decidedAt,
        last_message: { preview: GRANT_ROOM_LINE, role: 'assistant', created_at: decidedAt },
      };
      await page.route('**/api/auth/session', (route) =>
        route.fulfill(
          json({
            access_token: 'test-jwt-token',
            csrf_token: 'test-csrf-token',
            user: {
              id: 'user-123',
              user_id: 'user-123',
              email: 'test@example.com',
              display_name: 'Philippe',
              role: 'user',
              is_admin: false,
              user_status: 'active',
              tier: 'professional',
              tenant_id: 'user-123',
              manages_roster: true,
            },
          }),
        ),
      );
    },
  };

  await setupDashboardMocks(page, { role: 'user', skipProvidersRoute: true, managesRoster });

  await page.route('**/api/providers', (route) => route.fulfill(json(coachPlatforms)));
  // Onboarded, the group step behind them: the room is where they landed. A
  // coach who does not train has the athlete steps waived (carnet#1529).
  const waived = trains
    ? []
    : [
        { step_id: 'about_you', status: 'not_applicable' },
        { step_id: 'parq', status: 'not_applicable' },
      ];
  await page.route('**/api/me/onboarding-status', (route) =>
    route.fulfill(
      json({
        ...ONBOARDED_STATUS,
        steps: [...ONBOARDED_STATUS.steps, { step_id: 'coach_group', status: 'complete' }, ...waived],
        coaches_others: coachesOthers,
      }),
    ),
  );
  // The pending coach Home's checklist (carnet#1529): the room's group, no
  // athlete in it yet, no coaching platform; the seat ticks once granted.
  await page.route('**/api/me/coach-setup', (route) => {
    const extra = options.setup ?? {};
    const inviteDone = extra.athlete != null;
    const platformDone = extra.platform != null;
    const decided = [granted, inviteDone, platformDone];
    const current = decided.indexOf(false);
    const ids = ['seat', 'invite', 'platform'] as const;
    const body: CoachSetupResponse = {
      rows: ids.map((id, index) => ({ id, order: index + 1, done: decided[index], current: index === current })),
      done_count: decided.filter(Boolean).length,
      total: 3,
      groups: role === 'owner' ? [{ id: SEAT_GROUP_ID, name: SEAT_GROUP_NAME }] : [],
      athlete: null,
      platform: null,
      ...extra,
    };
    return route.fulfill(json(body));
  });
  // Once granted, Home is the coach Home: its roster read.
  await page.route('**/api/coach/roster**', (route) => route.fulfill(json(buildRoster(localToday(), 0))));
  await page.route('**/api/me/coach-access-request', async (route) => {
    const method = route.request().method();
    if (method === 'POST') {
      const body = (route.request().postDataJSON() as { group_id?: string } | null) ?? {};
      record.requests.push(body);
      latest = {
        ...PENDING_REQUEST,
        group_id: body.group_id ?? null,
        group_tenant_id: body.group_id ? 'tenant-1' : null,
        created_at: new Date().toISOString(),
      };
      await route.fulfill(json({ request: latest }, 201));
      return;
    }
    if (method === 'DELETE') {
      // The waiting request is deleted; nothing earlier stood on this account.
      record.cancels += 1;
      latest = null;
      await route.fulfill(json({ request: latest }));
      return;
    }
    await route.fulfill(json({ request: latest }));
  });

  await page.route('**/api/groups/permissions', (route) =>
    route.fulfill(json({ can_create: true, policy: 'everyone', weekly_digest: false })),
  );
  await page.route(`**/api/groups/${SEAT_GROUP_ID}`, (route) => route.fulfill(json(group)));
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
  // A coached room: the coach's TrainingPeaks links, or the athlete's own
  // record of the coach's reads and what they let the coach do.
  await page.route(`**/api/groups/${SEAT_GROUP_ID}/delegated-connections`, (route) =>
    route.fulfill(json({ connections: [], total: 0, viewer: options.coach === 'viewer' ? 'coach' : 'member' })),
  );
  await page.route(`**/api/groups/${SEAT_GROUP_ID}/delegated-connections/roster*`, (route) =>
    route.fulfill(
      json({
        provider: 'trainingpeaks',
        athletes: [
          { provider_athlete_id: '900001', display_name: 'Léa Gagnon', connection: null, suggested_member_user_id: 'user-lea' },
        ],
      }),
    ),
  );
  await page.route(`**/api/groups/${SEAT_GROUP_ID}/members/me/coach-reads`, (route) =>
    route.fulfill(json({ reads: [], retention_months: 12 })),
  );
  await page.route(`**/api/groups/${SEAT_GROUP_ID}/members/me/coach-consents`, (route) =>
    route.fulfill(
      json({
        calendar_write: false,
        calendar_write_at: null,
        calendar_write_supported: true,
        wellness_read: false,
        wellness_read_at: null,
        wellness_read_supported: true,
        wellness_read_skipped: null,
        wellness_notice_due: false,
      }),
    ),
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
    route.fulfill(json({ conversations: [conversation], total: 1, limit: 50, offset: 0 })),
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
