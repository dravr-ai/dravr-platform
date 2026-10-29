// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: E2E for a coach's per-athlete plan thread (carnet#1069/#1070): Group info → Work on <name>'s plan → the thread → Publish; and View <name>'s plan, read-only (carnet#1244)
// ABOUTME: Every route is mocked; the draft card is read back from the persisted transcript row, the way a reload draws it

import { test, expect, type Page } from '@playwright/test';
import { setupDashboardMocks, loginToDashboard, openGroups } from './test-helpers';

const GROUP_ID = 'group-1';
const GROUP_CONVERSATION_ID = 'conv-group-1';
const PLAN_CONVERSATION_ID = 'conv-plan-alice';
const COACH_ID = 'user-123';
const ATHLETE_ID = 'user-alice';

const mockCoaches = {
  agents: [
    {
      id: 'coach-marathon',
      title: 'Marathon Coach',
      description: 'Training for marathon runners',
      system_prompt: 'You are a marathon coach.',
      category: 'training',
      tags: ['running'],
      token_count: 200,
      is_favorite: false,
      use_count: 5,
      last_used_at: null,
      created_at: '2024-01-01T00:00:00Z',
      updated_at: '2024-01-01T00:00:00Z',
      is_system: true,
      handle: 'marathon-coach',
      visibility: 'tenant',
    },
  ],
  total: 1,
  metadata: { timestamp: '2024-06-01T10:00:00Z', api_version: 'v1' },
};

/** The coach's own thread for the group — the row that reaches Group info. */
const groupConversation = {
  id: GROUP_CONVERSATION_ID,
  title: 'Marathon Training 2026',
  agent_id: 'coach-marathon',
  agent_title: 'Marathon Coach',
  agent_handle: 'marathon-coach',
  group_id: GROUP_ID,
  group_name: 'Marathon Training 2026',
  message_count: 2,
  unread_count: 0,
  created_at: '2026-10-01T10:00:00Z',
  updated_at: '2026-10-15T15:00:00Z',
  last_message: { preview: 'Long run moved to Sunday.', role: 'assistant', created_at: '2026-10-15T15:00:00Z' },
};

/** The plan thread the server opens for Alice, bound to the group's agent and scoped to the group. */
const planConversation = {
  id: PLAN_CONVERSATION_ID,
  title: 'Alice Runner',
  agent_id: 'coach-marathon',
  agent_title: 'Marathon Coach',
  agent_handle: 'marathon-coach',
  group_id: null,
  group_name: null,
  subject_user_id: ATHLETE_ID,
  subject_group_id: GROUP_ID,
  subject_display_name: 'Alice Runner',
  message_count: 2,
  unread_count: 0,
  created_at: '2026-10-16T14:00:00Z',
  updated_at: '2026-10-16T14:05:01Z',
  last_message: { preview: 'Draft saved.', role: 'assistant', created_at: '2026-10-16T14:05:01Z' },
};

/** The group, with the signed-in user as its human coach: they hold no membership row. */
const groupDetail = {
  id: GROUP_ID,
  tenant_id: 'tenant-1',
  name: 'Marathon Training 2026',
  description: 'Preparing for the fall marathon',
  agent_id: 'coach-marathon',
  agent_title: 'Marathon Coach',
  agent_handle: 'marathon-coach',
  coach_user_id: COACH_ID,
  coach_display_name: 'Test Admin',
  owner_id: 'user-3',
  peer_data_sharing: false,
  respond_mode: 'all',
  digest_mode: 'off',
  max_members: 20,
  is_active: true,
  created_at: '2026-10-01T10:00:00Z',
  updated_at: '2026-10-15T15:00:00Z',
};

const members = [
  {
    id: 'member-alice',
    group_id: GROUP_ID,
    user_id: ATHLETE_ID,
    tenant_id: 'tenant-1',
    role: 'member',
    peer_sharing_consent: true,
    consent_given_at: '2026-10-02T12:00:00Z',
    joined_at: '2026-10-02T12:00:00Z',
    left_at: null,
    display_name: 'Alice Runner',
  },
  {
    id: 'member-bob',
    group_id: GROUP_ID,
    user_id: 'user-3',
    tenant_id: 'tenant-1',
    role: 'owner',
    peer_sharing_consent: false,
    consent_given_at: '2026-10-01T10:00:00Z',
    joined_at: '2026-10-01T10:00:00Z',
    left_at: null,
    display_name: 'Bob Cyclist',
  },
];

/** The plan card the platform projects for Alice's draft. */
const draftPlan = {
  goal_race: { name: 'Harricana 65K', date: '2027-09-11', discipline: 'trail', priority: 'A' },
  phases: [
    {
      kind: 'base',
      start: '2026-10-19',
      end: '2026-12-13',
      weeks: 8,
      purpose: 'build the aerobic floor',
      intent: 'easy volume, one long run',
      current: true,
    },
  ],
  weeks: [
    {
      week_start: '2026-10-19',
      focus: 'Easy volume',
      current: true,
      days: [
        { date: '2026-10-20', sport: 'run', workout: 'Easy 45 min', duration_min: 45, intensity: 'easy', rest: false },
        { date: '2026-10-25', sport: 'run', workout: 'Long run 1h40', duration_min: 100, intensity: 'easy', rest: false },
      ],
    },
  ],
  weeks_deferred: 6,
};

/** The draft's state, stamped the way chrono writes a Postgres timestamp: to the microsecond. */
const draftState = {
  draft_id: 'draft-7',
  updated_at: '2026-10-16T14:05:00.482913Z',
  conflicts: [],
  season_relaid: false,
};

/** The agent's reply that saved the draft, as the transcript stores it. */
const draftReply = {
  id: 'msg-plan-2',
  conversation_id: PLAN_CONVERSATION_ID,
  role: 'assistant',
  content: "Alice's base block is drafted. She won't see it until you publish.",
  created_at: '2026-10-16T14:05:01Z',
  scene_blocks: JSON.stringify([
    {
      kind: 'workout_plan',
      source_tool: 'save_training_plan',
      plan: draftPlan,
      subject: { user_id: ATHLETE_ID, display_name: 'Alice Runner' },
      draft: draftState,
    },
  ]),
};

const coachQuestion = {
  id: 'msg-plan-1',
  conversation_id: PLAN_CONVERSATION_ID,
  role: 'user',
  content: 'Draft an eight-week base block for Alice.',
  created_at: '2026-10-16T14:04:00Z',
};

function json(body: unknown, status = 200) {
  return { status, contentType: 'application/json', body: JSON.stringify(body) };
}

/** The coach's own membership row, for a coach who also belongs to the group they coach. */
const coachMembership = {
  id: 'member-coach',
  group_id: GROUP_ID,
  user_id: COACH_ID,
  tenant_id: 'tenant-1',
  role: 'member',
  peer_sharing_consent: false,
  consent_given_at: '2026-10-01T10:00:00Z',
  joined_at: '2026-10-01T10:00:00Z',
  left_at: null,
  display_name: 'Test Admin',
};

/** What the coach's actions sent, in order. */
interface Sent {
  opened: Array<{ path: string; body: unknown }>;
  published: unknown[];
  /** The read-only plan reads Group info made, with the group each named. */
  viewed: Array<{ path: string; groupId: string | null }>;
}

/** How the mocked server is set up beyond the default coach with no membership row. */
interface PlanThreadMockOptions {
  /** The coach also holds a membership row in the group they coach. */
  coachIsMember?: boolean;
  /** The role that membership row carries; a member's by default. */
  coachRole?: 'owner' | 'admin' | 'member';
  /** The refusal the athlete-thread route answers with, instead of opening the thread. */
  openRefusal?: { status: number; body: unknown };
}

async function setupPlanThreadMocks(page: Page, options: PlanThreadMockOptions = {}): Promise<Sent> {
  const sent: Sent = { opened: [], published: [], viewed: [] };
  let threadOpened = false;
  let published = false;
  const roster = options.coachIsMember
    ? [...members, { ...coachMembership, role: options.coachRole ?? coachMembership.role }]
    : members;

  await page.route('**/api/groups/permissions', (route) =>
    route.fulfill(json({ can_create: true, policy: 'everyone', weekly_digest: false })),
  );
  await page.route(`**/api/groups/${GROUP_ID}`, (route) => route.fulfill(json(groupDetail)));
  await page.route(`**/api/groups/${GROUP_ID}/members`, (route) => route.fulfill(json({ members: roster })));
  await page.route(`**/api/groups/${GROUP_ID}/delegated-connections`, (route) =>
    route.fulfill(json({ connections: [], total: 0, viewer: 'coach' })),
  );
  await page.route(`**/api/groups/${GROUP_ID}/delegated-connections/roster*`, (route) =>
    route.fulfill(json({ provider: 'trainingpeaks', athletes: [] })),
  );
  await page.route(`**/api/chat/groups/${GROUP_ID}/transcript*`, (route) =>
    route.fulfill(json({ group_id: GROUP_ID, members: [], entries: [] })),
  );
  await page.route('**/api/agents**', (route) => route.fulfill(json(mockCoaches)));
  await page.route('**/api/commands**', (route) => route.fulfill(json({ commands: [] })));

  // The list: the group's thread, and Alice's plan thread once it is opened.
  await page.route(/\/api\/chat\/conversations(\?.*)?$/, (route) => {
    const conversations = threadOpened ? [planConversation, groupConversation] : [groupConversation];
    return route.fulfill(json({ conversations, total: conversations.length, limit: 50, offset: 0 }));
  });

  await page.route(`**/api/chat/conversations/${GROUP_CONVERSATION_ID}/messages`, (route) =>
    route.fulfill(json({ messages: [] })),
  );
  await page.route(`**/api/chat/conversations/${PLAN_CONVERSATION_ID}/messages`, (route) =>
    route.fulfill(json({ messages: [coachQuestion, draftReply] })),
  );
  await page.route('**/api/chat/conversations/*/verdicts', (route) => route.fulfill(json({ verdicts: [], total: 0 })));
  await page.route(`**/api/chat/conversations/${GROUP_CONVERSATION_ID}/participants`, (route) =>
    route.fulfill(json({ participants: [] })),
  );
  await page.route(`**/api/chat/conversations/${PLAN_CONVERSATION_ID}/participants`, (route) =>
    route.fulfill(
      json({
        participants: [{ user_id: COACH_ID, role: 'owner', added_by: COACH_ID, added_at: '2026-10-16T14:00:00Z' }],
      }),
    ),
  );

  await page.route(`**/api/chat/athletes/${ATHLETE_ID}/thread`, (route) => {
    sent.opened.push({ path: new URL(route.request().url()).pathname, body: route.request().postDataJSON() });
    if (options.openRefusal) {
      return route.fulfill(json(options.openRefusal.body, options.openRefusal.status));
    }
    threadOpened = true;
    return route.fulfill(json(planConversation));
  });

  // Alice's live plan, read-only, as Group info reads it (carnet#1244).
  await page.route(`**/api/chat/athletes/${ATHLETE_ID}/plan*`, (route) => {
    const url = new URL(route.request().url());
    sent.viewed.push({ path: url.pathname, groupId: url.searchParams.get('group_id') });
    return route.fulfill(
      json({ athlete: { user_id: ATHLETE_ID, display_name: 'Alice Runner' }, today: '2026-10-16', live: draftPlan }),
    );
  });

  await page.route(`**/api/chat/conversations/${PLAN_CONVERSATION_ID}/subject-plan`, (route) =>
    route.fulfill(
      json({
        athlete: { user_id: ATHLETE_ID, display_name: 'Alice Runner' },
        today: '2026-10-16',
        live: null,
        draft: published ? null : draftPlan,
        draft_state: published ? null : draftState,
      }),
    ),
  );
  await page.route(`**/api/chat/conversations/${PLAN_CONVERSATION_ID}/subject-plan/publish`, (route) => {
    sent.published.push(route.request().postDataJSON());
    published = true;
    return route.fulfill(json({ plan_id: 'plan-2', superseded_plan_id: null, athlete_notified: true }));
  });

  return sent;
}

test.describe("A coach's plan thread for one athlete", () => {
  test('Group info → Work on the plan → the thread opens → the draft is published', async ({ page }) => {
    await setupDashboardMocks(page, { role: 'user' });
    const sent = await setupPlanThreadMocks(page);
    await loginToDashboard(page);
    await openGroups(page);

    // The group's thread, then Group info from its header.
    const groupRow = page.locator('[data-testid="conversation-row"]', { hasText: 'Marathon Training 2026' });
    await groupRow.getByRole('button').first().click();
    await page.getByTestId('conversation-header-title').click();
    await expect(page.getByTestId('group-info-panel')).toBeVisible({ timeout: 10000 });

    // The coach has connected nothing of their own, so their own group thread
    // asks them to.
    await expect(page.getByTestId('connect-provider-banner')).toBeVisible();

    // The coach works on Alice's plan; nobody else's row is the coach's own.
    await page.getByRole('button', { name: "Work on Alice Runner's plan" }).click();

    await expect.poll(() => sent.opened.length).toBe(1);
    expect(sent.opened[0]).toEqual({
      path: `/api/chat/athletes/${ATHLETE_ID}/thread`,
      body: { group_id: GROUP_ID },
    });

    // The thread opens where Groups threads live, named for the athlete.
    await page.waitForURL(new RegExp(`#chat/${PLAN_CONVERSATION_ID}$`));
    await expect(page.getByTestId('group-info-panel')).toHaveCount(0);
    await expect(page.getByTestId('thread-title')).toHaveText('Alice Runner');
    await expect(page.getByTestId('thread-subtitle')).toHaveText('Plan · Alice Runner');
    // Alice's plan reads Alice's data through her links: the coach is never
    // asked to connect a service of their own here (carnet#1191).
    await expect(page.getByTestId('connect-provider-banner')).toHaveCount(0);

    // Listed under Groups, with the plan glyph.
    const planRow = page.locator('[data-testid="conversation-row"]', { hasText: 'Alice Runner' });
    await expect(planRow.getByTestId('conversation-kind-glyph')).toHaveAttribute('data-kind', 'subject');

    // The draft card, read back from the transcript row.
    await expect(page.getByText('Harricana 65K')).toBeVisible();
    await expect(page.getByTestId('plan-draft-badge')).toHaveText("Draft · Alice Runner can't see it yet");

    await page.getByRole('button', { name: 'Publish to Alice Runner' }).click();

    await expect(page.getByTestId('plan-draft-published')).toHaveText('Published. Alice Runner has been notified.');
    expect(sent.published).toEqual([{ overwrite_athlete_changes: false }]);
    await expect(page.getByRole('button', { name: 'Discard draft' })).toHaveCount(0);
  });

  test("Group info → View the plan → the athlete's live plan, read-only → the thread from there", async ({
    page,
  }) => {
    await setupDashboardMocks(page, { role: 'user' });
    const sent = await setupPlanThreadMocks(page);
    await loginToDashboard(page);
    await openGroups(page);

    const groupRow = page.locator('[data-testid="conversation-row"]', { hasText: 'Marathon Training 2026' });
    await groupRow.getByRole('button').first().click();
    await page.getByTestId('conversation-header-title').click();
    await expect(page.getByTestId('group-info-panel')).toBeVisible({ timeout: 10000 });

    // Reading the plan opens no thread.
    await page.getByRole('button', { name: "View Alice Runner's plan" }).click();

    await expect(page.getByText('Read-only: the plan Alice Runner sees.')).toBeVisible();
    await expect(page.getByTestId('athlete-plan').getByText('Harricana 65K')).toBeVisible();
    expect(sent.viewed).toEqual([{ path: `/api/chat/athletes/${ATHLETE_ID}/plan`, groupId: GROUP_ID }]);
    expect(sent.opened).toEqual([]);
    // The read-only plan carries no draft control.
    await expect(page.getByRole('button', { name: 'Publish to Alice Runner' })).toHaveCount(0);

    // From the plan, the coach opens the thread where it changes.
    await page.getByTestId('athlete-plan-work-on').click();
    await expect.poll(() => sent.opened.length).toBe(1);
    await page.waitForURL(new RegExp(`#chat/${PLAN_CONVERSATION_ID}$`));
    await expect(page.getByTestId('athlete-plan')).toHaveCount(0);
    await expect(page.getByTestId('thread-title')).toHaveText('Alice Runner');
  });

  // At 1440 px the members table overflowed the Group info panel and cut the
  // Actions column off (carnet#1191). The widest row is an owner coach's: a
  // plan view, a plan thread, a role change and a removal on every other
  // member.
  test('Group info fits the members table, Actions column included, at 1440 px', async ({ page }) => {
    await page.setViewportSize({ width: 1440, height: 900 });
    await setupDashboardMocks(page, { role: 'user' });
    await setupPlanThreadMocks(page, { coachIsMember: true, coachRole: 'owner' });
    await loginToDashboard(page);
    await openGroups(page);

    const groupRow = page.locator('[data-testid="conversation-row"]', { hasText: 'Marathon Training 2026' });
    await groupRow.getByRole('button').first().click();
    await page.getByTestId('conversation-header-title').click();
    await expect(page.getByTestId('group-info-panel')).toBeVisible({ timeout: 10000 });

    const list = page.getByTestId('member-list');
    await expect(list.getByRole('button', { name: "Work on Alice Runner's plan" })).toBeVisible();

    // Nothing scrolls sideways inside the list.
    const overflow = await list.evaluate((el) => el.scrollWidth - el.clientWidth);
    expect(overflow).toBeLessThanOrEqual(0);

    // Every action on Alice's row sits inside the panel, the last one too.
    const panel = await page.getByTestId('group-info-panel').boundingBox();
    expect(panel).not.toBeNull();
    const aliceRow = list.locator('tr', { hasText: 'Alice Runner' });
    const actions = aliceRow.getByRole('button');
    await expect(actions).toHaveCount(4);
    for (const index of [0, 1, 2, 3]) {
      const box = await actions.nth(index).boundingBox();
      expect(box).not.toBeNull();
      if (box && panel) {
        expect(box.x).toBeGreaterThanOrEqual(panel.x);
        expect(box.x + box.width).toBeLessThanOrEqual(panel.x + panel.width + 0.5);
      }
    }

    // The join date and the sharing state read under the name.
    await expect(list.getByTestId(`member-detail-${ATHLETE_ID}`)).toContainText('Sharing on');
    await expect(list.getByTestId('member-detail-user-3')).toContainText('Sharing off');
  });

  test('A coach who is also a member works on every other member, and hears when one stopped sharing', async ({
    page,
  }) => {
    await setupDashboardMocks(page, { role: 'user' });
    const sent = await setupPlanThreadMocks(page, {
      coachIsMember: true,
      // The body the coaching gate answers with (CoachedAthleteRefusal::into_error).
      openRefusal: {
        status: 403,
        body: {
          code: 'PermissionDenied',
          message: "Alice Runner has stopped sharing their data with the coach of 'Marathon Training 2026'",
          details: { reason: 'coach_sharing_revoked' },
        },
      },
    });
    await loginToDashboard(page);
    await openGroups(page);

    const groupRow = page.locator('[data-testid="conversation-row"]', { hasText: 'Marathon Training 2026' });
    await groupRow.getByRole('button').first().click();
    await page.getByTestId('conversation-header-title').click();
    await expect(page.getByTestId('group-info-panel')).toBeVisible({ timeout: 10000 });

    // Every other member's row carries the action; the coach's own does not.
    await expect(page.getByRole('button', { name: "Work on Bob Cyclist's plan" })).toBeVisible();
    await expect(page.getByTestId(`member-work-on-plan-${COACH_ID}`)).toHaveCount(0);

    await page.getByRole('button', { name: "Work on Alice Runner's plan" }).click();

    // Retrying cannot help, so the coach is told why rather than asked to try again.
    await expect.poll(() => sent.opened.length).toBe(1);
    await expect(page.getByText("Alice Runner stopped sharing with you, so their plan can't be changed.")).toBeVisible();
    await expect(page.getByText("We couldn't open this plan thread. Please try again.")).toHaveCount(0);
    await expect(page.getByTestId('group-info-panel')).toBeVisible();
    expect(page.url()).not.toContain(PLAN_CONVERSATION_ID);
  });
});
