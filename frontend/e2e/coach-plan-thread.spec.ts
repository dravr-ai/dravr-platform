// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: E2E for a coach's per-athlete plan thread (carnet#1069/#1070), View <name>'s plan (carnet#1244), <name>'s thresholds (carnet#1561), the family editor (carnet#1515)
// ABOUTME: and the private lane in a one-athlete room (carnet#1249): Adjust plan from this → Only <agent> → Publish. Every route is mocked

import { test, expect, type Page } from '@playwright/test';
import { setupDashboardMocks, loginToDashboard, openGroups, PHYSIOLOGY_OFFERED } from './test-helpers';

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
        {
          date: '2026-10-20',
          sport: 'run',
          workout: 'Easy 45 min',
          duration_min: 45,
          intensity: 'easy',
          rest: false,
          family_sport: 'run',
        },
        {
          date: '2026-10-25',
          sport: 'run',
          workout: 'Long run 1h40',
          duration_min: 100,
          intensity: 'easy',
          rest: false,
          family_sport: 'run',
        },
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
    await page.getByRole('button', { name: 'Open their plan thread · Alice Runner' }).click();

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
    await page.getByRole('button', { name: 'View plan · Alice Runner' }).click();

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
  // Actions column off (carnet#1191). The cards that replaced it (carnet#1527)
  // must hold the widest set: an owner coach's plan thread, plan view and
  // `⋯` menu on every other member.
  test("Group info fits each athlete's card, every action included, at 1440 px", async ({ page }) => {
    await page.setViewportSize({ width: 1440, height: 900 });
    await setupDashboardMocks(page, { role: 'user' });
    await setupPlanThreadMocks(page, { coachIsMember: true, coachRole: 'owner' });
    await loginToDashboard(page);
    await openGroups(page);

    const groupRow = page.locator('[data-testid="conversation-row"]', { hasText: 'Marathon Training 2026' });
    await groupRow.getByRole('button').first().click();
    await page.getByTestId('conversation-header-title').click();
    await expect(page.getByTestId('group-info-panel')).toBeVisible({ timeout: 10000 });

    const athletes = page.getByTestId('group-info-athletes');
    await expect(athletes.getByRole('button', { name: 'Open their plan thread · Alice Runner' })).toBeVisible();

    // Nothing scrolls sideways inside the athletes' section.
    const overflow = await athletes.evaluate((el) => el.scrollWidth - el.clientWidth);
    expect(overflow).toBeLessThanOrEqual(0);

    // Every action on Alice's card sits inside the panel, the last one too.
    const panel = await page.getByTestId('group-info-panel').boundingBox();
    expect(panel).not.toBeNull();
    const aliceCard = page.getByTestId(`coach-athlete-${ATHLETE_ID}`);
    const actions = aliceCard.getByRole('button');
    expect(await actions.count()).toBeGreaterThanOrEqual(3);
    for (let index = 0; index < (await actions.count()); index += 1) {
      const box = await actions.nth(index).boundingBox();
      expect(box).not.toBeNull();
      if (box && panel) {
        expect(box.x).toBeGreaterThanOrEqual(panel.x);
        expect(box.x + box.width).toBeLessThanOrEqual(panel.x + panel.width + 0.5);
      }
    }

    // The role and the sharing state read under the name.
    await expect(athletes.getByTestId(`member-detail-${ATHLETE_ID}`)).toContainText('Sharing on');
    await expect(athletes.getByTestId('member-detail-user-3')).toContainText('Sharing off');
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
    await expect(page.getByRole('button', { name: 'Open their plan thread · Bob Cyclist' })).toBeVisible();
    await expect(page.getByTestId(`member-work-on-plan-${COACH_ID}`)).toHaveCount(0);

    await page.getByRole('button', { name: 'Open their plan thread · Alice Runner' }).click();

    // Retrying cannot help, so the coach is told why rather than asked to try again.
    await expect.poll(() => sent.opened.length).toBe(1);
    await expect(page.getByText("Alice Runner stopped sharing with you, so their plan can't be changed.")).toBeVisible();
    await expect(page.getByText("We couldn't open this plan thread. Please try again.")).toHaveCount(0);
    await expect(page.getByTestId('group-info-panel')).toBeVisible();
    expect(page.url()).not.toContain(PLAN_CONVERSATION_ID);
  });
});

// ---------------------------------------------------------------------------
// carnet#1561: from Group info the coach reads an athlete's thresholds, worded
// for a coach, and sets one for the group they coach the athlete in.
// ---------------------------------------------------------------------------

/** Alice's FTP as her coach reads it: her own value. */
const aliceFtp = {
  anchor: 'ftp',
  unit: 'watts',
  value: 285,
  low: null,
  high: null,
  origin: 'athlete',
  as_of: '2026-10-20',
  method: null,
  confidence: 'medium',
  value_text: '285 W',
  range_text: null,
  source_text: "the athlete's own value",
};

test.describe("A coach sets an athlete's thresholds from Group info", () => {
  test('Group info → Thresholds → the coach changes the FTP, sent for the group, and reads it back as set by a coach', async ({
    page,
  }) => {
    await setupDashboardMocks(page, { role: 'user' });
    await setupPlanThreadMocks(page);
    const read: Array<{ path: string; groupId: string | null }> = [];
    const saved: Array<{ path: string; groupId: string | null; body: unknown }> = [];
    await page.route(`**/api/chat/athletes/${ATHLETE_ID}/physiology**`, (route) => {
      const request = route.request();
      const url = new URL(request.url());
      if (request.method() === 'PUT') {
        saved.push({ path: url.pathname, groupId: url.searchParams.get('group_id'), body: request.postDataJSON() });
        const set = { ...aliceFtp, value: 292, value_text: '292 W', origin: 'coach', source_text: 'set by a coach' };
        return route.fulfill(json({ sports: [{ sport: 'ride', anchors: [set] }], offered: PHYSIOLOGY_OFFERED }));
      }
      read.push({ path: url.pathname, groupId: url.searchParams.get('group_id') });
      return route.fulfill(json({ sports: [{ sport: 'ride', anchors: [aliceFtp] }], offered: PHYSIOLOGY_OFFERED }));
    });
    await loginToDashboard(page);
    await openGroups(page);

    const groupRow = page.locator('[data-testid="conversation-row"]', { hasText: 'Marathon Training 2026' });
    await groupRow.getByRole('button').first().click();
    await page.getByTestId('conversation-header-title').click();
    await expect(page.getByTestId('group-info-panel')).toBeVisible({ timeout: 10000 });

    await page.getByRole('button', { name: 'Thresholds · Alice Runner' }).click();
    await expect(page.getByText("Alice Runner's thresholds")).toBeVisible();
    await expect(page.getByTestId('physiology-ride-ftp-source')).toHaveText("the athlete's own value");
    expect(read).toEqual([{ path: `/api/chat/athletes/${ATHLETE_ID}/physiology`, groupId: GROUP_ID }]);

    await page.getByTestId('physiology-ride-ftp-edit').click();
    const editor = page.getByTestId('physiology-editor');
    await editor.getByTestId('physiology-editor-value').fill('292');
    await editor.getByTestId('physiology-editor-kind').selectOption('measured');
    await editor.getByTestId('physiology-editor-save').click();

    await expect(page.getByTestId('athlete-physiology-saved')).toHaveText('Saved. Alice Runner is told.');
    await expect(page.getByTestId('physiology-ride-ftp-source')).toHaveText('set by a coach');
    expect(saved).toEqual([
      {
        path: `/api/chat/athletes/${ATHLETE_ID}/physiology/ride/ftp`,
        groupId: GROUP_ID,
        body: {
          value: 292,
          low: null,
          high: null,
          pace_per: null,
          kind: 'measured',
          confidence: 'medium',
          method: null,
          measured_on: null,
        },
      },
    ]);
  });
});

// ---------------------------------------------------------------------------
// carnet#1515: in the plan thread, each day of the plan card opens the
// "build from family" editor; the kernel's preview follows the values, a
// value outside the evidence earns a finding and still saves (D3), and the
// save lands in the coach's draft.
// ---------------------------------------------------------------------------

/** The family the thread's agent offers, as `GET …/subject-plan/families` answers it. */
const familyCatalogue = {
  families: [
    {
      slug: 'vo2max_short',
      name: 'VO2max — short intervals',
      purpose: 'vo2max_short',
      purpose_label: 'short-interval VO2max work',
      sports: ['ride', 'run'],
      default_preset: '30_15',
      presets: [
        {
          key: '30_15',
          name: '30/15 (Rønnestad)',
          sports: ['ride', 'run'],
          params: {
            ride: { sets: { min: 2, max: 4, default: 3 }, reps: { min: 10, max: 13, default: 13 } },
            run: { sets: { min: 2, max: 3, default: 2 }, reps: { min: 10, max: 13, default: 13 } },
          },
        },
      ],
    },
  ],
};

/** The run form's reps ceiling, which a build past it is measured against. */
const REPS_LIMIT = 13;

/** The preview the kernel answers for a run build of `reps` per set: a finding past the ceiling. */
function sessionPreview(reps: number) {
  const outside = reps > REPS_LIMIT;
  return {
    family: 'vo2max_short',
    preset: '30_15',
    sport: 'run',
    duration_min: 40,
    steps: [
      { label: 'Warm-up', duration_seconds: 900, target_zone: 'easy' },
      { label: 'Work', duration_seconds: 30, target_zone: '100-105% vVO2max', repeat: reps, repeat_group: 1 },
      { label: 'Cool-down', duration_seconds: 600, target_zone: 'easy' },
    ],
    derived: {
      sets: 2,
      reps,
      work_seconds: 30,
      rest_seconds: 15,
      ratio: 2,
      work_pct: 102,
      on_time_minutes: reps,
      total_minutes: 40,
      aerobic_max: 'vVO2max',
    },
    findings: outside
      ? [
          {
            code: 'param_out_of_range',
            text: `${reps} reps per set is outside this format's tested range (limit ${REPS_LIMIT}).`,
            param: 'reps',
          },
        ]
      : [],
    ...(outside ? { outside_evidence: 'Outside the evidence' } : {}),
  };
}

/** The draft once the built day is saved over 20 October. */
const savedDraftPlan = {
  ...draftPlan,
  weeks: [
    {
      ...draftPlan.weeks[0],
      days: [
        {
          ...draftPlan.weeks[0].days[0],
          workout: 'VO2max — short intervals — 30/15 (Rønnestad)',
          duration_min: 40,
          intensity: '100-105% vVO2max',
          template_slug: 'vo2max_short',
          template_source: 'catalogue',
          template_preset: '30_15',
          template_params: { sets: 2, reps: 20 },
          outside_evidence: 'Outside the evidence',
        },
        draftPlan.weeks[0].days[1],
      ],
    },
  ],
};

/** What the editor sent: the previews it asked for and the day it saved. */
interface EditorSent {
  previews: unknown[];
  saved: Array<{ path: string; body: unknown }>;
}

async function setupFamilyEditorMocks(page: Page): Promise<EditorSent> {
  const sent: EditorSent = { previews: [], saved: [] };
  const base = `**/api/chat/conversations/${PLAN_CONVERSATION_ID}/subject-plan`;
  const savedRead = {
    athlete: { user_id: ATHLETE_ID, display_name: 'Alice Runner' },
    today: '2026-10-16',
    live: null,
    draft: savedDraftPlan,
    draft_state: { ...draftState, updated_at: '2026-10-16T14:20:00.120000Z' },
  };

  await page.route(`${base}/families**`, (route) => route.fulfill(json(familyCatalogue)));
  await page.route(`${base}/session-preview**`, (route) => {
    const body = route.request().postDataJSON() as { params?: { reps?: number } };
    sent.previews.push(body);
    return route.fulfill(json(sessionPreview(body.params?.reps ?? REPS_LIMIT)));
  });
  await page.route(`${base}/draft/days/**`, (route) => {
    sent.saved.push({ path: new URL(route.request().url()).pathname, body: route.request().postDataJSON() });
    return route.fulfill(json(savedRead));
  });
  // Once the day is saved, the plan thread reads the draft as the save left it.
  await page.route(base, (route) => (sent.saved.length > 0 ? route.fulfill(json(savedRead)) : route.fallback()));
  return sent;
}

test.describe("A coach builds a day of the athlete's plan from a workout family", () => {
  test('Build from family → a value past the evidence earns a finding → Save puts the day in the draft', async ({
    page,
  }) => {
    await setupDashboardMocks(page, { role: 'user' });
    await setupPlanThreadMocks(page);
    const sent = await setupFamilyEditorMocks(page);
    await loginToDashboard(page);
    await openGroups(page);

    const groupRow = page.locator('[data-testid="conversation-row"]', { hasText: 'Marathon Training 2026' });
    await groupRow.getByRole('button').first().click();
    await page.getByTestId('conversation-header-title').click();
    await expect(page.getByTestId('group-info-panel')).toBeVisible({ timeout: 10000 });
    await page.getByRole('button', { name: 'Open their plan thread · Alice Runner' }).click();
    await page.waitForURL(new RegExp(`#chat/${PLAN_CONVERSATION_ID}$`));
    await expect(page.getByTestId('plan-draft-badge')).toHaveText("Draft · Alice Runner can't see it yet");

    // Every day of the card offers the editor; the first opens on its run.
    const build = page.getByRole('button', { name: 'Build from family' });
    await expect(build).toHaveCount(2);
    await build.first().click();
    const editor = page.getByTestId('session-editor');
    await expect(editor.getByTestId('session-editor-family')).toHaveValue('vo2max_short');
    await expect(editor.getByRole('radio', { name: 'Run' })).toBeChecked();
    await expect(editor.getByTestId('session-editor-param-sets')).toHaveValue('2');
    await expect(editor.getByTestId('session-editor-range-sets')).toHaveText('Evidence range 2–3');

    // The preview at the preset's defaults: the kernel's steps, nothing found.
    await expect(editor.getByTestId('session-editor-findings')).toContainText("Inside the family's evidence.");
    await expect(editor.getByTestId('session-editor-derived')).toContainText('% of vVO2max');

    // A value past the evidence is kept as typed, marked, and measured.
    await editor.getByTestId('session-editor-param-reps').fill('20');
    await expect(editor.getByTestId('session-editor-range-reps')).toHaveAttribute('data-outside', 'true');
    await expect(editor.getByTestId('session-editor-findings')).toContainText(
      "20 reps per set is outside this format's tested range (limit 13).",
    );
    await expect(editor.getByTestId('session-editor-outside-evidence')).toHaveText('Outside the evidence');
    expect(sent.previews.at(-1)).toEqual({
      family: 'vo2max_short',
      preset: '30_15',
      sport: 'run',
      params: { sets: 2, reps: 20 },
      date: '2026-10-20',
    });

    // The finding never holds Save back: the day goes into the coach's draft.
    await editor.getByTestId('session-editor-save').click();
    await expect(page.getByText('Saved to your draft. Alice Runner sees it only once you publish.')).toBeVisible();
    await expect(page.getByTestId('session-editor')).toHaveCount(0);
    expect(sent.saved).toEqual([
      {
        path: `/api/chat/conversations/${PLAN_CONVERSATION_ID}/subject-plan/draft/days/2026-10-20`,
        body: { family: 'vo2max_short', preset: '30_15', sport: 'run', params: { sets: 2, reps: 20 } },
      },
    ]);

    // The thread's card showed the draft before the save, so the draft as it
    // stands now is drawn with its controls, the built day in it.
    const open = page.getByTestId('chat-plan-thread-open-draft');
    await expect(open).toContainText('VO2max — short intervals — 30/15 (Rønnestad)');
    await expect(open.getByRole('button', { name: 'Publish to Alice Runner' })).toBeEnabled();
  });
});

// ---------------------------------------------------------------------------
// carnet#1249: in a room of one coach, one athlete and the agent, the coach
// drafts the athlete's plan with the agent privately, inside the room.
// ---------------------------------------------------------------------------

/** The room's one athlete, named to its coach alone. */
const soloRoomDetail = { ...groupDetail, solo_athlete: { user_id: ATHLETE_ID, display_name: 'Alice Runner' } };

/** Alice's own words in the room — the message the coach adjusts the plan from. */
const aliceRoomEntry = {
  id: 'entry-alice-1',
  speaker: 'member',
  withheld: false,
  own: false,
  author_user_id: ATHLETE_ID,
  author_display_name: 'Alice Runner',
  content: "Legs felt heavy in this morning's swim. Is Thursday's long run still a good idea?",
  message_id: null,
  created_at: '2026-10-16T07:12:00Z',
};

const PRIVATE_QUESTION = 'Keep Thursday easy and move the long run to Sunday.';

/** The coach's private question as the work thread stores it, the quote copied by the server. */
const privateQuestion = {
  ...coachQuestion,
  content: PRIVATE_QUESTION,
  quote: {
    entry_id: aliceRoomEntry.id,
    author_user_id: ATHLETE_ID,
    author_display_name: 'Alice Runner',
    content: aliceRoomEntry.content,
    created_at: aliceRoomEntry.created_at,
  },
};

/** What the private send and the room reads came to. */
interface PrivateLaneSent {
  /** Every turn posted, with the conversation it went to and its body. */
  turns: Array<{ conversationId: string; body: { content?: string; quote_entry_id?: string } }>;
  opened: unknown[];
  published: unknown[];
  /** How often the room transcript was read. */
  roomReads: number;
}

async function setupPrivateLaneMocks(page: Page): Promise<PrivateLaneSent> {
  const sent: PrivateLaneSent = { turns: [], opened: [], published: [], roomReads: 0 };
  let threadOpened = false;
  let turnSent = false;
  let published = false;

  await page.route('**/api/groups/permissions', (route) =>
    route.fulfill(json({ can_create: true, policy: 'everyone', weekly_digest: false })),
  );
  await page.route(`**/api/groups/${GROUP_ID}`, (route) => route.fulfill(json(soloRoomDetail)));
  await page.route(`**/api/groups/${GROUP_ID}/members`, (route) => route.fulfill(json({ members: [members[0]] })));
  await page.route(`**/api/chat/groups/${GROUP_ID}/transcript*`, (route) => {
    sent.roomReads += 1;
    return route.fulfill(json({ group_id: GROUP_ID, members: [], entries: [aliceRoomEntry] }));
  });
  await page.route('**/api/agents**', (route) => route.fulfill(json(mockCoaches)));
  await page.route('**/api/commands**', (route) => route.fulfill(json({ commands: [] })));

  // The list: the room, and the coach's work thread about Alice once the first private send opened it.
  await page.route(/\/api\/chat\/conversations(\?.*)?$/, (route) => {
    const conversations = threadOpened ? [planConversation, groupConversation] : [groupConversation];
    return route.fulfill(json({ conversations, total: conversations.length, limit: 50, offset: 0 }));
  });

  await page.route('**/api/chat/conversations/*/messages**', async (route, request) => {
    const conversationId = new URL(request.url()).pathname.split('/')[4];
    if (request.method() === 'POST') {
      sent.turns.push({ conversationId, body: request.postDataJSON() });
      turnSent = true;
      const done = {
        turn_id: 'turn-private-1',
        user_message: privateQuestion,
        assistant: {
          message: { ...draftReply, scene_blocks: undefined },
          blocks: [
            { type: 'prose', text: draftReply.content },
            {
              type: 'workout_plan',
              plan: draftPlan,
              subject: { user_id: ATHLETE_ID, display_name: 'Alice Runner' },
              draft: draftState,
            },
          ],
          finish_reason: 'stop',
        },
        conversation_updated_at: draftReply.created_at,
        telemetry: {
          model: 'm',
          provider_name: 'p',
          tool_calls_count: 1,
          tools_called: ['save_training_plan'],
          execution_time_ms: 2_000,
        },
      };
      const frames: Array<[string, unknown]> = [
        ['progress', { kind: 'tool', id: 'c1', title: 'save_training_plan', status: 'Completed' }],
        ['delta', { delta: draftReply.content }],
        ['done', done],
      ];
      const body = frames.map(([event, data]) => `event: ${event}\ndata: ${JSON.stringify(data)}\n\n`).join('');
      await route.fulfill({ status: 200, contentType: 'text/event-stream', body });
      return;
    }
    // The room conversation holds none of the coach's own rows; the work
    // thread holds the private turn once it was sent.
    const messages = conversationId === PLAN_CONVERSATION_ID && turnSent ? [privateQuestion, draftReply] : [];
    await route.fulfill(json({ messages }));
  });
  await page.route('**/api/chat/conversations/*/verdicts', (route) => route.fulfill(json({ verdicts: [], total: 0 })));
  await page.route('**/api/chat/conversations/*/read', (route) => route.fulfill(json({})));
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
    sent.opened.push(route.request().postDataJSON());
    threadOpened = true;
    return route.fulfill(json(planConversation));
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
    return route.fulfill(
      json({ plan_id: 'plan-2', superseded_plan_id: null, athlete_notified: true, posted_in_room: true }),
    );
  });

  return sent;
}

test.describe("A coach's private turn in a one-athlete room", () => {
  test('Adjust plan from this → Only the agent → a private reply with the draft → Publish', async ({ page }, testInfo) => {
    await setupDashboardMocks(page, { role: 'user' });
    const sent = await setupPrivateLaneMocks(page);
    await loginToDashboard(page);
    await openGroups(page);

    const groupRow = page.locator('[data-testid="conversation-row"]', { hasText: 'Marathon Training 2026' });
    await groupRow.getByRole('button').first().click();

    // The coach's composer starts at Everyone, named by the label it shows
    // (WCAG 2.5.3); the name carries the state, so no pressed state on top.
    const toggle = page.getByTestId('composer-audience-switch');
    await expect(toggle).toHaveText('Everyone');
    await expect(toggle).toHaveAccessibleName('Everyone. Switch to only Marathon Coach');
    await expect(toggle).not.toHaveAttribute('aria-pressed');

    // Alice's message offers the coach a private plan change.
    const aliceRow = page.locator('[data-testid="message-row"]', { hasText: 'Legs felt heavy' });
    await aliceRow.hover();
    await aliceRow.getByRole('button', { name: 'Adjust plan from this' }).click();

    await expect(page.getByTestId('composer-quote')).toContainText("From Alice Runner's message");
    await expect(toggle).toHaveAccessibleName('Only Marathon Coach. Switch to everyone');
    await expect(toggle).toHaveText('Only Marathon Coach');
    const field = page.locator('textarea[data-composer="true"]');
    await expect(field).toBeFocused();
    await expect(field).toHaveAttribute('placeholder', "Ask Marathon Coach to change Alice Runner's plan. Alice Runner won't see this.");

    await field.fill(PRIVATE_QUESTION);
    await page.getByRole('button', { name: 'Send message' }).click();

    // The first private send opens the work thread through this room, and the
    // turn goes there with the quote — never to the room's conversation.
    await expect.poll(() => sent.turns.length).toBe(1);
    expect(sent.opened).toEqual([{ group_id: GROUP_ID }]);
    expect(sent.turns[0]).toEqual({
      conversationId: PLAN_CONVERSATION_ID,
      body: { content: PRIVATE_QUESTION, quote_entry_id: aliceRoomEntry.id },
    });

    // The private stretch, inline in the room: the line, the dashed question
    // with its quote, the tagged reply and its draft card.
    await expect(page.getByTestId('private-lane-divider')).toHaveText('Only you and Marathon Coach see what follows');
    const question = page.locator('[data-testid="message-row"][data-audience="private"]', { hasText: PRIVATE_QUESTION });
    await expect(question.getByTestId('message-quote')).toContainText("From Alice Runner's message");
    const reply = page.locator('[data-testid="message-row"][data-audience="private"]', {
      hasText: "Alice's base block is drafted.",
    });
    await expect(reply.getByTestId('private-reply-tag')).toHaveText('Only you');
    await expect(reply.getByTestId('plan-draft-badge')).toHaveText("Draft · Alice Runner can't see it yet");
    await expect(page.getByTestId('composer-quote')).toHaveCount(0);

    for (const theme of ['light', 'dark'] as const) {
      await page.evaluate((dark) => document.documentElement.classList.toggle('dark', dark), theme === 'dark');
      // Fast-forward the 200 ms colour transitions, so the shot shows the theme it names.
      await page.screenshot({ path: testInfo.outputPath(`private-lane-${theme}.png`), animations: 'disabled' });
    }
    await page.evaluate(() => document.documentElement.classList.remove('dark'));

    const readsBeforePublish = sent.roomReads;
    await reply.getByRole('button', { name: 'Publish to Alice Runner' }).click();

    await expect(reply.getByTestId('plan-draft-published')).toHaveText('Published. Alice Runner has been notified.');
    expect(sent.published).toEqual([{ overwrite_athlete_changes: false }]);
    // The publish posts in the room, which is read again to show it.
    await expect.poll(() => sent.roomReads).toBeGreaterThan(readsBeforePublish);
    expect(sent.turns.every((turn) => turn.conversationId === PLAN_CONVERSATION_ID)).toBe(true);
  });
});
