// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: E2E for group management after the Groups tab: the group row, its header, and Group info
// ABOUTME: Who runs the group, roster rows and their menu, invites, autosaved settings, consent, the folded week, quiet exits, links

import { test, expect, type Page } from '@playwright/test';
import type {
  GroupHealthFlag,
  GroupStatsResponse,
  GroupWeeklyReportResponse,
  JoinHeldInviteResponse,
} from '@pierre/shared-types';
import { setupDashboardMocks, loginToDashboard, openGroups } from './test-helpers';
import { expectNoCollidingText, useFrench } from './text-collision';
import { describeLayoutFailures, measurePageLayout } from './layout-gate';

// ============================================================================
// Mock Data
// ============================================================================

const GROUP_ID = 'group-1';
const CONVERSATION_ID = 'conv-group-1';

const mockCoaches = {
  agents: [
    {
      id: 'coach-marathon',
      title: 'Marathon Coach',
      description: 'Training for marathon runners',
      system_prompt: 'You are a marathon coach.',
      category: 'training',
      tags: ['running', 'marathon'],
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

/** The member's own group-scoped conversation — the row that reaches Group info. */
const groupConversation = {
  id: CONVERSATION_ID,
  title: 'Marathon Training 2026',
  agent_id: 'coach-marathon',
  agent_title: 'Marathon Coach',
  agent_handle: 'marathon-coach',
  group_id: GROUP_ID,
  group_name: 'Marathon Training 2026',
  message_count: 4,
  unread_count: 0,
  created_at: '2026-03-01T10:00:00Z',
  updated_at: '2026-03-20T15:00:00Z',
  last_message: {
    preview: 'Long run moved to Sunday.',
    role: 'assistant',
    created_at: '2026-03-20T15:00:00Z',
  },
};

const mockGroupDetail = {
  id: GROUP_ID,
  tenant_id: 'tenant-1',
  name: 'Marathon Training 2026',
  description: 'Preparing for the fall marathon',
  agent_id: 'coach-marathon',
  agent_title: 'Marathon Coach',
  agent_handle: 'marathon-coach',
  coach_user_id: null,
  coach_display_name: null,
  owner_id: 'user-123',
  peer_data_sharing: false,
  respond_mode: 'all',
  digest_mode: 'off',
  max_members: 20,
  is_active: true,
  created_at: '2026-03-01T10:00:00Z',
  updated_at: '2026-03-20T15:00:00Z',
};

function buildMockMembers(currentUserRole: 'owner' | 'admin' | 'member') {
  return {
    members: [
      {
        id: 'member-1',
        group_id: GROUP_ID,
        user_id: 'user-123',
        tenant_id: 'tenant-1',
        role: currentUserRole,
        peer_sharing_consent: false,
        consent_given_at: '2026-03-01T10:00:00Z',
        joined_at: '2026-03-01T10:00:00Z',
        left_at: null,
        display_name: 'Test User',
      },
      {
        id: 'member-2',
        group_id: GROUP_ID,
        user_id: 'user-2',
        tenant_id: 'tenant-1',
        role: 'member',
        peer_sharing_consent: true,
        consent_given_at: '2026-03-05T12:00:00Z',
        joined_at: '2026-03-05T12:00:00Z',
        left_at: null,
        display_name: 'Alice Runner',
      },
      {
        id: 'member-3',
        group_id: GROUP_ID,
        user_id: 'user-3',
        tenant_id: 'tenant-1',
        role: 'admin',
        peer_sharing_consent: false,
        consent_given_at: '2026-03-10T09:00:00Z',
        joined_at: '2026-03-10T09:00:00Z',
        left_at: null,
        display_name: 'Bob Cyclist',
      },
    ],
  };
}

const mockInvites = {
  invites: [
    {
      id: 'invite-1',
      group_id: GROUP_ID,
      tenant_id: 'tenant-1',
      code: 'MRT2026X',
      created_by: 'user-123',
      expires_at: '2027-06-01T00:00:00Z',
      max_uses: 10,
      use_count: 3,
      is_active: true,
      approves_signup: false,
      created_at: '2026-03-15T10:00:00Z',
    },
  ],
};

/**
 * The member's own record as `GET /api/groups/:id/members/me/coach-reads`
 * serialises it: newest first, a tool by its name and each read made outside
 * a tool call by the label the server records for it.
 */
const mockCoachReads = {
  retention_months: 12,
  reads: [
    { coach_name: 'Casey Coach', tool: 'coach_thread', data_class: 'wellness', read_at: '2026-11-12T14:05:00Z', served_to_assistant: false },
    { coach_name: 'Casey Coach', tool: 'coach_thread', data_class: 'training', read_at: '2026-11-12T14:05:00Z', served_to_assistant: false },
    { coach_name: 'Casey Coach', tool: 'get_activities', data_class: 'training', read_at: '2026-11-12T14:04:00Z', served_to_assistant: true },
    { coach_name: 'Casey Coach', tool: 'plan_view', data_class: 'training', read_at: '2026-11-11T18:20:00Z', served_to_assistant: false },
    { coach_name: 'Casey Coach', tool: 'plan_command', data_class: 'training', read_at: '2026-11-11T07:45:00Z', served_to_assistant: false },
    { coach_name: null, tool: 'roster', data_class: 'training', read_at: '2026-11-10T08:30:00Z', served_to_assistant: false },
    { coach_name: 'Casey Coach', tool: 'group_room', data_class: 'training', read_at: '2026-11-10T08:00:00Z', served_to_assistant: false },
    { coach_name: 'Casey Coach', tool: 'group_stats', data_class: 'training', read_at: '2026-11-09T19:10:00Z', served_to_assistant: false },
    { coach_name: 'Casey Coach', tool: 'group_report', data_class: 'training', read_at: '2026-11-09T19:05:00Z', served_to_assistant: false },
    { coach_name: 'Casey Coach', tool: 'group_health', data_class: 'training', read_at: '2026-11-09T19:00:00Z', served_to_assistant: false },
    { coach_name: 'Casey Coach', tool: 'weekly_digest', data_class: 'training', read_at: '2026-11-09T13:00:00Z', served_to_assistant: false },
  ],
};

/** Health flags exactly as `GET /api/groups/:id/health` serialises them. */
const mockHealthFlags: GroupHealthFlag[] = [
  {
    user_id: 'user-456',
    display_name: 'Alice Runner',
    flag_type: 'volume_drop',
    severity: 'warning',
    evidence: { kind: 'volume_below_group', pct_below: 35 },
  },
  {
    user_id: 'user-789',
    display_name: 'Bob Cyclist',
    flag_type: 'inactive',
    severity: 'warning',
    evidence: { kind: 'inactive_days', days: 11 },
  },
];

const mockStats: GroupStatsResponse = {
  stats: {
    total_members: 5,
    active_members: 4,
    avg_weekly_volume_km: 38.5,
    avg_ctl: 52.0,
    flagged_members: 1,
    weekly_trend: 'improving',
  },
  scope: { visible_members: 5, roster_members: 5 },
};

// ============================================================================
// Setup Helpers
// ============================================================================

interface GroupMockOptions {
  userGroupRole?: 'owner' | 'admin' | 'member';
  /** The tenant tier flag gating the weekly report + health flags panel. */
  weeklyDigest?: boolean;
  /** The signed-in account holds coach access (carnet#1650's claim needs it). */
  managesRoster?: boolean;
}

/** Every turn the chat surface sent, in order. */
interface SentTurn {
  conversationId: string;
  content: string;
}

async function setupGroupMocks(page: Page, options: GroupMockOptions = {}): Promise<SentTurn[]> {
  const { userGroupRole = 'owner', weeklyDigest = true } = options;
  const mockMembers = buildMockMembers(userGroupRole);
  // The group's owner follows the viewer's role: ownership is what offers a
  // coachless group's coach seat (carnet#1522), so a member viewer must not
  // also be served as the owner.
  const groupDetail = { ...mockGroupDetail, owner_id: userGroupRole === 'owner' ? 'user-123' : 'user-3' };
  const sent: SentTurn[] = [];

  // A hard navigation (the invite link is a real path, not a hash) remounts
  // the app, which restores its session from the cookie before anything else.
  await page.route('**/api/auth/session', async (route) => {
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({
        access_token: 'test-jwt-token',
        csrf_token: 'test-csrf-token',
        user: {
          id: 'user-123',
          user_id: 'user-123',
          email: 'admin@test.com',
          display_name: 'Test Admin',
          role: 'user',
          is_admin: false,
          user_status: 'active',
          tier: 'professional',
          tenant_id: 'user-123',
        },
      }),
    });
  });

  await page.route('**/api/groups/permissions', async (route) => {
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({ can_create: true, policy: 'everyone', weekly_digest: weeklyDigest }),
    });
  });

  await page.route(`**/api/groups/${GROUP_ID}`, async (route) => {
    if (route.request().method() === 'GET') {
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify(groupDetail),
      });
    } else if (route.request().method() === 'PUT') {
      const body = route.request().postDataJSON();
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({ ...groupDetail, ...body }),
      });
    } else if (route.request().method() === 'DELETE') {
      await route.fulfill({ status: 204 });
    } else {
      await route.fallback();
    }
  });

  await page.route(`**/api/groups/${GROUP_ID}/members`, async (route) => {
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify(mockMembers),
    });
  });

  // Member role change and removal (more specific path — registered after the
  // generic members route so LIFO gives it priority).
  await page.route(`**/api/groups/${GROUP_ID}/members/**`, async (route) => {
    if (route.request().method() === 'DELETE') {
      await route.fulfill({ status: 204 });
    } else if (route.request().method() === 'PUT') {
      await route.fulfill({ status: 200, contentType: 'application/json', body: '{}' });
    } else {
      await route.fallback();
    }
  });

  // The member's record of their human coach's reads (carnet#1445), shown in
  // Group info once a coach is attached: a tool read and one read of each
  // source the server records outside a tool call.
  await page.route(`**/api/groups/${GROUP_ID}/members/me/coach-reads`, async (route) => {
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify(mockCoachReads),
    });
  });

  await page.route(`**/api/groups/${GROUP_ID}/invites`, async (route) => {
    if (route.request().method() === 'GET') {
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify(mockInvites),
      });
    } else if (route.request().method() === 'POST') {
      await route.fulfill({
        status: 201,
        contentType: 'application/json',
        body: JSON.stringify({
          id: 'invite-new',
          group_id: GROUP_ID,
          tenant_id: 'tenant-1',
          code: 'NEWCODE1',
          created_by: 'user-123',
          expires_at: null,
          max_uses: null,
          use_count: 0,
          is_active: true,
          approves_signup: false,
          created_at: new Date().toISOString(),
        }),
      });
    } else {
      await route.fallback();
    }
  });

  await page.route(`**/api/groups/${GROUP_ID}/invites/**`, async (route) => {
    if (route.request().method() === 'DELETE') {
      await route.fulfill({ status: 204 });
    } else {
      await route.fallback();
    }
  });

  await page.route(`**/api/groups/${GROUP_ID}/stats`, async (route) => {
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify(mockStats),
    });
  });

  await page.route(`**/api/groups/${GROUP_ID}/health`, async (route) => {
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({
        flags: mockHealthFlags,
        total: mockHealthFlags.length,
        scope: mockStats.scope,
      }),
    });
  });

  await page.route(`**/api/groups/${GROUP_ID}/report`, async (route) => {
    const body: GroupWeeklyReportResponse = {
      report: {
        stats: mockStats.stats,
        fresh_members: [{ user_id: 'user-456', display_name: 'Alice Runner', form_pct: 12, tsb: 9 }],
      },
      scope: mockStats.scope,
    };
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify(body),
    });
  });

  // The room transcript hangs off the chat routes, not the group ones.
  await page.route(`**/api/chat/groups/${GROUP_ID}/transcript*`, async (route) => {
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({ entries: [], total: 0 }),
    });
  });

  await page.route('**/api/groups/group-*/leave', async (route) => {
    await route.fulfill({ status: 204 });
  });

  await page.route('**/api/groups/group-*/members/me/consent', async (route) => {
    await route.fulfill({ status: 200, contentType: 'application/json', body: '{}' });
  });

  await page.route('**/api/agents**', async (route) => {
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify(mockCoaches),
    });
  });

  await page.route('**/api/commands**', async (route) => {
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({ commands: [] }),
    });
  });

  await page.route(`**/api/chat/conversations/${CONVERSATION_ID}/messages`, async (route) => {
    if (route.request().method() === 'POST') {
      sent.push({
        conversationId: CONVERSATION_ID,
        content: (route.request().postDataJSON() as { content: string }).content,
      });
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({
          turn_id: 'turn-1',
          user_message: {
            id: 'm1',
            role: 'user',
            content: '',
            created_at: new Date().toISOString(),
          },
          assistant: {
            message: {
              id: 'm2',
              role: 'assistant',
              content: 'Joined.',
              created_at: new Date().toISOString(),
            },
            blocks: [],
            finish_reason: 'command',
          },
          conversation_updated_at: new Date().toISOString(),
          telemetry: {
            model: 'command',
            provider_name: 'command',
            tool_calls_count: 0,
            tools_called: [],
            execution_time_ms: 3,
          },
        }),
      });
      return;
    }
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({ messages: [] }),
    });
  });

  await page.route(`**/api/chat/conversations/${CONVERSATION_ID}/verdicts`, async (route) => {
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({ verdicts: [], total: 0 }),
    });
  });

  await page.route(`**/api/chat/conversations/${CONVERSATION_ID}/participants`, async (route) => {
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({ participants: [] }),
    });
  });

  await page.route(/\/api\/chat\/conversations(\?.*)?$/, async (route, request) => {
    if (request.method() === 'POST') {
      await route.fulfill({
        status: 201,
        contentType: 'application/json',
        body: JSON.stringify({ ...groupConversation, id: CONVERSATION_ID }),
      });
      return;
    }
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({
        conversations: [groupConversation],
        total: 1,
        limit: 50,
        offset: 0,
      }),
    });
  });

  return sent;
}

/**
 * Sign in and open Group info from the group row's header. `extraRoutes`
 * registers after the group mocks, so a spec's own routes take priority.
 */
async function openGroupInfo(
  page: Page,
  options: GroupMockOptions = {},
  extraRoutes?: (page: Page) => Promise<void>,
) {
  await setupDashboardMocks(page, { role: 'user', managesRoster: options.managesRoster ?? false });
  const sent = await setupGroupMocks(page, options);
  if (extraRoutes) await extraRoutes(page);
  await loginToDashboard(page);
  await page.waitForSelector('aside', { timeout: 10000 });
  // Sign-in lands on Home; the group's thread is in Chat.
  await openGroups(page);

  // The group row carries its kind glyph, and opens the thread.
  const row = page.locator('[data-testid="conversation-row"]', {
    hasText: 'Marathon Training 2026',
  });
  await expect(row.getByTestId('conversation-kind-glyph')).toBeVisible({ timeout: 10000 });
  await row.getByRole('button').first().click();

  await page.getByTestId('conversation-header-title').click();
  await expect(page.getByTestId('group-info-panel')).toBeVisible({ timeout: 10000 });
  return sent;
}

// ============================================================================
// Group info — roster
// ============================================================================

test.describe('Group info — roster', () => {
  test('the header names the group and opens its info drawer', async ({ page }) => {
    await openGroupInfo(page);

    await expect(page.getByTestId('conversation-header-title')).toHaveText(/Marathon Training 2026/);
    await expect(page.getByRole('dialog', { name: 'Group info' })).toBeVisible();
    await expect(page.getByTestId('group-info-name')).toHaveText('Marathon Training 2026');
    await expect(page.getByTestId('group-info-description')).toHaveText(
      'Preparing for the fall marathon',
    );
  });

  test('lists every member with their role, one row each', async ({ page }) => {
    await openGroupInfo(page);

    // The owner waiting on the seat reads themselves in the coach slot, the others in the list.
    await expect(page.getByTestId('group-info-coach-seat')).toHaveText('You · not the coach yet');
    const roster = page.getByTestId('member-list');
    await expect(roster.getByText('Alice Runner')).toBeVisible();
    await expect(roster.getByText('Bob Cyclist')).toBeVisible();
    await expect(roster.getByText('Admin', { exact: true })).toBeVisible();
    await expect(roster.getByText('Member', { exact: true })).toBeVisible();
    await expect(roster.getByText('Test User')).toHaveCount(0);
    await expect(page.getByRole('table')).toHaveCount(0);
  });

  test('names the AI agent and the human coach above the members', async ({ page }) => {
    await openGroupInfo(page, { userGroupRole: 'member' }, async (p) => {
      await p.route(`**/api/groups/${GROUP_ID}`, async (route) => {
        await route.fulfill({
          status: 200,
          contentType: 'application/json',
          body: JSON.stringify({
            ...mockGroupDetail,
            coach_user_id: 'user-coach',
            coach_display_name: 'Casey Coach',
          }),
        });
      });
    });

    const agent = page.getByTestId('group-info-agent');
    await expect(agent).toContainText('Marathon Coach');
    await expect(agent.getByTestId('group-info-agent-handle')).toHaveText('@marathon-coach');
    await expect(agent).toContainText('AI agent');
    const coach = page.getByTestId('group-info-coach-row');
    await expect(coach).toContainText('Casey Coach');
    // The role badge alone reads exactly "Coach"; the name merely contains it.
    await expect(coach.getByText('Coach', { exact: true })).toBeVisible();
    await expect(coach).not.toContainText('(you)');
    await expect(page.getByTestId('group-info-coach-badge')).toHaveText('Coach Casey Coach');
    await expect(page.getByTestId('group-info-no-coach')).toHaveCount(0);
    // The coach's id is never printed, only their name.
    await expect(page.getByTestId('group-info-panel')).not.toContainText('user-coach');
  });

  test('says the group has no human coach yet', async ({ page }) => {
    await openGroupInfo(page, { userGroupRole: 'member' });

    await expect(page.getByTestId('group-info-agent')).toContainText('Marathon Coach');
    await expect(page.getByTestId('group-info-no-coach')).toHaveText('No human coach in this group yet.');
    await expect(page.getByTestId('group-info-coach-row')).toHaveCount(0);
  });

  // carnet#1650: an owner who holds coach access takes the coachless group's seat in one tap.
  test('an owner with coach access claims the coachless group and becomes its coach', async ({ page }) => {
    let coached = false;
    const claims: string[] = [];
    await openGroupInfo(page, { userGroupRole: 'owner', managesRoster: true }, async (p) => {
      await p.route('**/api/groups/group-1', async (route) => {
        if (route.request().method() !== 'GET') return route.fallback();
        await route.fulfill({
          status: 200,
          contentType: 'application/json',
          body: JSON.stringify({
            ...mockGroupDetail,
            owner_id: 'user-123',
            coach_user_id: coached ? 'user-123' : null,
            coach_display_name: coached ? 'Test Admin' : null,
          }),
        });
      });
      await p.route('**/api/groups/group-1/coach', async (route) => {
        if (route.request().method() !== 'PUT') return route.fallback();
        claims.push('PUT');
        coached = true;
        await route.fulfill({
          status: 200,
          contentType: 'application/json',
          body: JSON.stringify({
            group: { ...mockGroupDetail, owner_id: 'user-123', coach_user_id: 'user-123', coach_display_name: 'Test Admin' },
          }),
        });
      });
    });

    const block = page.getByTestId('group-info-claim-coach');
    await expect(block).toContainText('This group has no coach yet');
    await expect(page.getByTestId('coach-seat-block')).toHaveCount(0);
    await block.getByTestId('group-info-claim-coach-action').click();

    await expect(page.getByText("You're now this group's coach.")).toBeVisible();
    await expect(page.getByTestId('group-info-claim-coach')).toHaveCount(0);
    expect(claims).toEqual(['PUT']);
  });

  test('an owner can promote and remove a member', async ({ page }) => {
    await openGroupInfo(page, { userGroupRole: 'owner' });

    // Role and removal sit behind the row's ⋯; nothing on the row is filled red.
    await expect(page.getByTestId('group-info-panel').locator('.btn-danger')).toHaveCount(0);
    await page.getByRole('button', { name: 'More actions for Alice Runner' }).click();
    await page.getByRole('menuitem', { name: 'Promote to admin' }).click();
    await expect(page.getByText(/Role updated/i)).toBeVisible({ timeout: 5000 });

    await page.getByRole('button', { name: 'More actions for Alice Runner' }).click();
    await page.getByRole('menuitem', { name: 'Remove from group…' }).click();
    await expect(page.getByText('Remove Alice Runner?')).toBeVisible();
    await page.getByRole('button', { name: 'Remove' }).last().click();
    await expect(page.getByText('Member removed')).toBeVisible({ timeout: 5000 });
  });

  test('a plain member gets no roster actions and no settings form', async ({ page }) => {
    await openGroupInfo(page, { userGroupRole: 'member' });

    await expect(page.getByTestId('group-info-people')).toBeVisible();
    await expect(page.getByRole('button', { name: /More actions for/ })).toHaveCount(0);
    await expect(page.getByRole('button', { name: /Group settings/ })).toHaveCount(0);
  });
});

// ============================================================================
// Group info — invites
// ============================================================================

test.describe('Group info — invites', () => {
  test('lists the existing invites with their use count and a copy control', async ({ page }) => {
    await openGroupInfo(page, { userGroupRole: 'owner' });

    await expect(page.getByText('MRT2026X')).toBeVisible({ timeout: 5000 });
    await expect(page.getByText('Uses: 3 / 10')).toBeVisible();
    await expect(
      page.getByRole('button', { name: 'Copy invite link to clipboard' }),
    ).toBeVisible();
  });

  test('creates a new invite', async ({ page }) => {
    await openGroupInfo(page, { userGroupRole: 'owner' });

    await page.getByRole('button', { name: 'New invite link' }).click();
    await expect(page.getByTestId('invite-create-form')).toBeVisible();
    await page.getByRole('button', { name: 'Create' }).click();
    await expect(page.getByText('Invite created')).toBeVisible({ timeout: 5000 });
  });
});

// ============================================================================
// Group info — settings, consent, analytics
// ============================================================================

test.describe('Group info — settings and consent', () => {
  test('an admin renames the group, saved as the field is left', async ({ page }) => {
    const updates: Array<Record<string, unknown>> = [];
    await openGroupInfo(page, { userGroupRole: 'owner' });
    page.on('request', (request) => {
      if (request.method() === 'PUT' && request.url().endsWith(`/api/groups/${GROUP_ID}`)) {
        updates.push(request.postDataJSON() as Record<string, unknown>);
      }
    });

    await page.getByRole('button', { name: /Group settings/ }).click();
    const nameInput = page.getByLabel('Group name', { exact: true });
    await nameInput.clear();
    await nameInput.fill('Renamed Group');
    await nameInput.press('Tab');
    await expect(page.getByText('Settings saved')).toBeVisible({ timeout: 5000 });
    expect(updates).toEqual([{ name: 'Renamed Group' }]);
  });

  test('binds the peer-consent switch to the caller own membership row', async ({ page }) => {
    const consentBodies: Array<Record<string, unknown>> = [];
    // The switch shows while the group shares at all.
    await openGroupInfo(page, { userGroupRole: 'member' }, async (p) => {
      await p.route(`**/api/groups/${GROUP_ID}`, async (route) => {
        await route.fulfill({
          status: 200,
          contentType: 'application/json',
          body: JSON.stringify({ ...mockGroupDetail, owner_id: 'user-3', peer_data_sharing: true }),
        });
      });
    });
    await page.route('**/api/groups/group-*/members/me/consent', async (route) => {
      consentBodies.push(route.request().postDataJSON() as Record<string, unknown>);
      await route.fulfill({ status: 200, contentType: 'application/json', body: '{}' });
    });

    const toggle = page.getByTestId('peer-consent-switch');
    await expect(toggle).not.toBeChecked();

    await toggle.click();
    await expect.poll(() => consentBodies.length).toBe(1);
    expect(consentBodies[0]).toEqual({ consent: true });
  });

  test('binds the coach-sharing switch to the caller own membership row', async ({ page }) => {
    const sharingBodies: Array<Record<string, unknown>> = [];
    // An athlete's switch: a member, whom the coachless group does not offer its coach seat.
    await openGroupInfo(page, { userGroupRole: 'member' }, async (p) => {
      await p.route('**/api/groups/group-*/members/me/coach-sharing', async (route) => {
        if (route.request().method() === 'PUT') {
          const body = route.request().postDataJSON() as Record<string, unknown>;
          sharingBodies.push(body);
          await route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify(body) });
        } else {
          // Joining granted it.
          await route.fulfill({
            status: 200,
            contentType: 'application/json',
            body: JSON.stringify({ consent: true, assistant_notice_due: false }),
          });
        }
      });
    });

    const toggle = page.getByTestId('coach-sharing-switch');
    await expect(toggle).toBeChecked();

    await toggle.click();
    await expect.poll(() => sharingBodies.length).toBe(1);
    expect(sharingBodies[0]).toEqual({ consent: false });
    await expect(toggle).not.toBeChecked();
  });

  test("a member reads their coach's reads of them, each made outside a tool call in words (carnet#1445)", async ({
    page,
  }) => {
    await openGroupInfo(page, { userGroupRole: 'member' }, async (p) => {
      await p.route(`**/api/groups/${GROUP_ID}`, async (route) => {
        await route.fulfill({
          status: 200,
          contentType: 'application/json',
          body: JSON.stringify({ ...mockGroupDetail, coach_user_id: 'user-coach', coach_display_name: 'Casey Coach' }),
        });
      });
    });

    const log = page.getByTestId('coach-read-log');
    const rows = log.getByTestId('coach-read-row');
    await expect(rows).toHaveCount(mockCoachReads.reads.length);
    // A tool is named as it is.
    await expect(rows.nth(2).getByTestId('coach-read-tool')).toHaveText('get_activities');
    // Only the read the server marked as served to the coach's outside AI tool says so.
    await expect(log.getByTestId('coach-read-served-to-assistant')).toHaveCount(1);
    await expect(rows.nth(2).getByTestId('coach-read-served-to-assistant')).toBeVisible();
    // A read the server records under a label of its own never shows the label.
    for (const [index, label] of [
      [0, 'coach_thread'],
      [3, 'plan_view'],
      [4, 'plan_command'],
      [5, 'roster'],
      [6, 'group_room'],
      [7, 'group_stats'],
      [8, 'group_report'],
      [9, 'group_health'],
      [10, 'weekly_digest'],
    ] as const) {
      await expect(rows.nth(index).getByTestId('coach-read-tool')).not.toHaveText(label);
    }
  });

  test('shows the weekly report and one row per flagged member, folded under This week', async ({ page }) => {
    await openGroupInfo(page);

    await expect(page.getByTestId('group-report-summary')).toHaveCount(0);
    await page.getByRole('button', { name: 'This week' }).click();

    await expect(page.getByTestId('group-report-summary')).toHaveText(
      '4/5 members active this week, averaging 38.5 km each.',
    );
    await expect(page.getByTestId('group-report-highlight')).toHaveText([
      'Alice Runner: fresh form (+12% of chronic load, TSB +9)',
    ]);
    // The concerns are the health flags, one line per flag.
    await expect(page.getByTestId('group-report-concern')).toHaveText([
      'Alice Runner: Weekly volume 35% below the group average',
      'Bob Cyclist: No activity for 11 days',
    ]);
    await expect(page.getByTestId('group-report-recommendation')).toHaveText([
      'Members in deep fatigue, with form below -30% of their chronic load: 1. Worth a look at their recovery.',
      'Group volume is up on last week — make sure recovery keeps pace.',
    ]);
    await expect(page.getByTestId('group-health-flag-row')).toHaveCount(2);
    await expect(page.getByTestId('group-health-flag-row').nth(1)).toContainText('No activity for 11 days');
    await expect(page.getByText('Health flags (2)')).toBeVisible();
  });

  test('a reader on imperial units reads the group volume in miles (carnet#835)', async ({ page }) => {
    await openGroupInfo(page, {}, async (p) => {
      await p.route('**/api/me/units**', (route) =>
        route.fulfill({
          status: 200,
          contentType: 'application/json',
          body: JSON.stringify({ units: 'imperial' }),
        }),
      );
    });

    // 38.5 km is 23.9 mi.
    await page.getByRole('button', { name: 'This week' }).click();
    await expect(page.getByTestId('group-report-summary')).toHaveText(
      '4/5 members active this week, averaging 23.9 mi each.',
    );
    const stats = page.getByTestId('group-info-stats');
    await expect(stats).toContainText('23.9mi');
    await expect(stats).not.toContainText('km');
  });

  test('withholds the weekly report when the tenant tier does not include it', async ({ page }) => {
    await openGroupInfo(page, { weeklyDigest: false });

    await page.getByRole('button', { name: 'This week' }).click();
    await expect(page.getByTestId('group-insights-tier-locked')).toBeVisible();
    await expect(page.getByTestId('group-report-summary')).toHaveCount(0);
  });
});

// ============================================================================
// Group info — the exits
// ============================================================================

test.describe('Group info — exits', () => {
  test('a member leaves the group', async ({ page }) => {
    await openGroupInfo(page, { userGroupRole: 'member' });

    await expect(page.getByTestId('group-info-delete')).toHaveCount(0);
    await expect(page.getByTestId('group-info-leave')).toHaveText('Leave this group…');
    await page.getByTestId('group-info-leave').click();
    await expect(page.getByText('You will need a new invite to rejoin')).toBeVisible();
    await page.getByRole('button', { name: 'Leave group' }).last().click();
    await expect(page.getByText('Left group')).toBeVisible({ timeout: 5000 });
  });

  test('an owner archives the group', async ({ page }) => {
    await openGroupInfo(page, { userGroupRole: 'owner' });

    await expect(page.getByTestId('group-info-leave')).toHaveCount(0);
    await expect(page.getByTestId('group-info-delete')).toHaveText('Delete this group…');
    await page.getByTestId('group-info-delete').click();
    await expect(page.getByText('This will permanently archive')).toBeVisible();
    await page.getByRole('button', { name: 'Delete group' }).last().click();
    await expect(page.getByText('Group deleted')).toBeVisible({ timeout: 5000 });
  });
});

// ============================================================================
// Group info — coaching-platform links through the group's coach
// ============================================================================

/** A link as the group's delegated-connection routes serialise it: TrainingPeaks, which offers no consent, unless `overrides` says otherwise. */
function delegatedLink(
  status: 'proposed' | 'confirmed',
  memberUserId: string,
  memberName: string,
  overrides: Record<string, unknown> = {},
) {
  return {
    id: 'dc-1',
    group_id: GROUP_ID,
    provider: 'trainingpeaks',
    coach_user_id: 'user-coach',
    coach_display_name: 'Casey Coach',
    member_user_id: memberUserId,
    member_display_name: memberName,
    provider_athlete_id: '900001',
    provider_athlete_name: 'Alex Athlete',
    status,
    proposed_at: '2026-09-24T08:00:00Z',
    confirmed_at: status === 'confirmed' ? '2026-09-24T09:00:00Z' : null,
    read_refused: null,
    calendar_write: false,
    calendar_write_supported: false,
    wellness_read: false,
    wellness_read_supported: false,
    wellness_read_skipped: null,
    metadata: { timestamp: '2026-09-24T09:00:00Z', api_version: '1.0' },
    ...overrides,
  };
}

const LIST_URL = `**/api/groups/${GROUP_ID}/delegated-connections`;

test.describe('Group info — coaching-platform links', () => {
  test('the coach links a roster athlete to a member and waits for them', async ({ page }) => {
    const proposals: Array<Record<string, unknown>> = [];
    let linked = false;

    await openGroupInfo(page, {}, async (p) => {
      // The signed-in user is the group's human coach and holds no membership.
      await p.route(`**/api/groups/${GROUP_ID}`, async (route) => {
        await route.fulfill({
          status: 200,
          contentType: 'application/json',
          body: JSON.stringify({
            ...mockGroupDetail,
            coach_user_id: 'user-123',
            coach_display_name: 'Test Admin',
            owner_id: 'user-3',
          }),
        });
      });
      await p.route(`**/api/groups/${GROUP_ID}/members`, async (route) => {
        const members = buildMockMembers('member').members.filter((m) => m.user_id !== 'user-123');
        await route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify({ members }) });
      });
      await p.route(LIST_URL, async (route) => {
        if (route.request().method() === 'POST') {
          proposals.push(route.request().postDataJSON() as Record<string, unknown>);
          linked = true;
          await route.fulfill({
            status: 201,
            contentType: 'application/json',
            body: JSON.stringify(delegatedLink('proposed', 'user-2', 'Alice Runner')),
          });
          return;
        }
        await route.fulfill({
          status: 200,
          contentType: 'application/json',
          body: JSON.stringify({
            connections: linked ? [delegatedLink('proposed', 'user-2', 'Alice Runner')] : [],
            total: linked ? 1 : 0,
            viewer: 'coach',
          }),
        });
      });
      await p.route(`${LIST_URL}/roster*`, async (route) => {
        await route.fulfill({
          status: 200,
          contentType: 'application/json',
          body: JSON.stringify({
            provider: 'trainingpeaks',
            athletes: [
              {
                provider_athlete_id: '900001',
                display_name: 'Alex Athlete',
                connection: linked ? delegatedLink('proposed', 'user-2', 'Alice Runner') : null,
                suggested_member_user_id: 'user-2',
              },
            ],
          }),
        });
      });
    });

    const section = page.getByTestId('delegation-section');
    await expect(section).toBeVisible();
    await expect(page.getByTestId('group-info-coach-badge')).toHaveText('You coach this group');
    // The coach meets their athletes first, then these links, then the agent.
    await expect(page.getByTestId('group-info-athletes')).toBeVisible();
    await expect(page.getByTestId('group-info-agent')).toContainText('Marathon Coach');
    // A coach holds no membership: nothing to leave, archive or consent to, and
    // of the settings only where the weekly digest goes.
    await expect(page.getByTestId('group-info-leave')).toHaveCount(0);
    await expect(page.getByTestId('group-info-delete')).toHaveCount(0);
    await expect(page.getByTestId('peer-consent-card')).toHaveCount(0);
    await expect(page.getByTestId('group-info-remove-coach')).toHaveCount(0);
    await page.getByRole('button', { name: /Group settings/ }).click();
    await expect(page.getByTestId('group-digest-mode')).toBeVisible();
    await expect(page.getByLabel('Group name')).toHaveCount(0);

    const row = section.getByTestId('delegation-roster-row-900001');
    await expect(row).toContainText('Alex Athlete');
    await expect(row.getByTestId('delegation-member-select-900001')).toHaveValue('user-2');
    await row.getByTestId('delegation-propose-900001').click();

    await expect.poll(() => proposals.length).toBe(1);
    expect(proposals[0]).toEqual({
      provider: 'trainingpeaks',
      provider_athlete_id: '900001',
      member_user_id: 'user-2',
    });
    await expect(row).toContainText('Waiting for Alice Runner to confirm');

    // The section holds at phone width: nothing scrolls sideways.
    await page.setViewportSize({ width: 390, height: 844 });
    const report = await measurePageLayout(page);
    expect(report.documentOverflowPx).toBe(0);
    const overflow = await section.evaluate((el) => el.scrollWidth - el.clientWidth);
    expect(overflow).toBeLessThanOrEqual(0);
  });

  test('a member confirms the link their coach proposed, then unlinks it', async ({ page }) => {
    let state: 'proposed' | 'confirmed' | 'ended' = 'proposed';
    const confirms: string[] = [];
    const deletes: string[] = [];

    await openGroupInfo(page, { userGroupRole: 'member' }, async (p) => {
      await p.route(`**/api/groups/${GROUP_ID}`, async (route) => {
        await route.fulfill({
          status: 200,
          contentType: 'application/json',
          body: JSON.stringify({
            ...mockGroupDetail,
            coach_user_id: 'user-coach',
            coach_display_name: 'Casey Coach',
          }),
        });
      });
      await p.route(LIST_URL, async (route) => {
        const connections = state === 'ended' ? [] : [delegatedLink(state, 'user-123', 'Test User')];
        await route.fulfill({
          status: 200,
          contentType: 'application/json',
          body: JSON.stringify({ connections, total: connections.length, viewer: 'member' }),
        });
      });
      await p.route(`${LIST_URL}/dc-1/confirm`, async (route) => {
        confirms.push(route.request().method());
        state = 'confirmed';
        await route.fulfill({
          status: 200,
          contentType: 'application/json',
          body: JSON.stringify(delegatedLink('confirmed', 'user-123', 'Test User')),
        });
      });
      await p.route(`${LIST_URL}/dc-1`, async (route) => {
        deletes.push(route.request().method());
        state = 'ended';
        await route.fulfill({ status: 204 });
      });
    });

    const request = page.getByTestId('delegation-request');
    await expect(request).toContainText('Casey Coach coaches you on TrainingPeaks as Alex Athlete');
    await request.getByTestId('delegation-confirm').click();

    await expect.poll(() => confirms).toEqual(['POST']);
    const linkedRow = page.getByTestId('delegation-linked');
    await expect(linkedRow).toContainText("Your TrainingPeaks workouts are read through Casey Coach's account.");
    await expect(page.getByText('TrainingPeaks linked')).toBeVisible();

    await linkedRow.getByTestId('delegation-unlink-dc-1').click();
    await expect(page.getByRole('heading', { name: 'Unlink TrainingPeaks?' })).toBeVisible();
    await page.getByRole('button', { name: 'Unlink', exact: true }).last().click();

    await expect.poll(() => deletes).toEqual(['DELETE']);
    await expect(page.getByTestId('delegation-linked')).toHaveCount(0);
  });

  test('a member grants calendar writes as they confirm an Intervals.icu link, then shares wellness in the coach block', async ({
    page,
  }) => {
    const INTERVALS = { provider: 'intervals_icu', calendar_write_supported: true, wellness_read_supported: true };
    let link = delegatedLink('proposed', 'user-123', 'Test User', INTERVALS);
    const confirmBodies: unknown[] = [];
    const consentBodies: unknown[] = [];
    // The member's consents live on their coach relationship; the link mirrors them.
    // A coached member is offered both, whatever they link.
    let consents = {
      calendar_write: false,
      calendar_write_at: null,
      calendar_write_supported: true,
      wellness_read: false,
      wellness_read_at: null,
      wellness_read_supported: true,
      wellness_read_skipped: null,
      wellness_notice_due: false,
    };

    await openGroupInfo(page, { userGroupRole: 'member' }, async (p) => {
      await p.route(`**/api/groups/${GROUP_ID}`, async (route) => {
        await route.fulfill({
          status: 200,
          contentType: 'application/json',
          body: JSON.stringify({ ...mockGroupDetail, coach_user_id: 'user-coach', coach_display_name: 'Casey Coach' }),
        });
      });
      await p.route(LIST_URL, async (route) => {
        await route.fulfill({
          status: 200,
          contentType: 'application/json',
          body: JSON.stringify({ connections: [link], total: 1, viewer: 'member' }),
        });
      });
      await p.route(`${LIST_URL}/dc-1/confirm`, async (route) => {
        const body = route.request().postDataJSON() as Record<string, boolean>;
        confirmBodies.push(body);
        link = delegatedLink('confirmed', 'user-123', 'Test User', { ...INTERVALS, ...body });
        consents = { ...consents, ...body };
        await route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify(link) });
      });
      await p.route(`**/api/groups/${GROUP_ID}/members/me/coach-consents`, async (route) => {
        if (route.request().method() === 'PUT') {
          const body = route.request().postDataJSON() as Record<string, boolean>;
          consentBodies.push({ method: 'PUT', body });
          consents = { ...consents, ...body };
          link = { ...link, ...body };
        }
        await route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify(consents) });
      });
    });

    const request = page.getByTestId('delegation-request');
    const calendar = request.getByRole('checkbox', { name: /Let Casey Coach put workouts on your Intervals\.icu calendar/ });
    const wellness = request.getByRole('checkbox', { name: /Let Casey Coach see your sleep, HRV/ });
    await expect(calendar).not.toBeChecked();
    await expect(wellness).not.toBeChecked();
    await calendar.check();
    await request.getByTestId('delegation-confirm').click();

    await expect.poll(() => confirmBodies).toEqual([{ calendar_write: true, wellness_read: false }]);
    await expect(page.getByTestId('delegation-linked')).toBeVisible();
    // Once confirmed, the switches are the coach relationship's, one block for the group.
    await expect(page.getByTestId('delegation-linked').getByRole('checkbox')).toHaveCount(0);
    const block = page.getByTestId('coach-consents');
    await expect(block.getByTestId('coach-consent-calendar_write')).toBeChecked();
    await expect(block.getByTestId('coach-consent-calendar-step')).toContainText('intervals.icu/athletes');
    await expect(block.getByTestId('coach-consent-wellness_read')).not.toBeChecked();

    await block.getByTestId('coach-consent-wellness_read').check();
    await expect.poll(() => consentBodies).toEqual([{ method: 'PUT', body: { wellness_read: true } }]);
    await expect(page.getByText('Sharing updated')).toBeVisible();
    await expect(block.getByTestId('coach-consent-wellness_read')).toBeChecked();

    // Turning it off asks first, since it deletes the wellness Dravr read.
    await block.getByTestId('coach-consent-wellness_read').click();
    // The group info is a dialog of its own: the confirm is the one that asks.
    const stop = page.getByRole('dialog').filter({ hasText: 'Stop sharing wellness?' });
    await expect(stop).toContainText(
      "Casey Coach stops seeing your sleep, HRV, resting heart rate, weight, readiness and daily notes. Dravr removes what it synced through Casey Coach's Intervals.icu account; what your own apps sync stays in your account.",
    );
    await stop.getByRole('button', { name: 'Stop sharing' }).click();
    await expect
      .poll(() => consentBodies)
      .toEqual([
        { method: 'PUT', body: { wellness_read: true } },
        { method: 'PUT', body: { wellness_read: false } },
      ]);
    await expect(block.getByTestId('coach-consent-wellness_read')).not.toBeChecked();

    // The consents hold at phone width: nothing scrolls sideways.
    await page.setViewportSize({ width: 390, height: 844 });
    const report = await measurePageLayout(page);
    expect(report.documentOverflowPx).toBe(0);
  });

  test('a member whose wellness grant predates its widening to every app is told, and keeps sharing', async ({
    page,
  }) => {
    const consentBodies: unknown[] = [];
    let consents = {
      calendar_write: false,
      calendar_write_at: null,
      calendar_write_supported: true,
      wellness_read: true,
      wellness_read_at: '2026-10-01T08:00:00Z',
      wellness_read_supported: true,
      wellness_read_skipped: null,
      wellness_notice_due: true,
    };

    await openGroupInfo(page, { userGroupRole: 'member' }, async (p) => {
      await p.route(`**/api/groups/${GROUP_ID}`, async (route) => {
        await route.fulfill({
          status: 200,
          contentType: 'application/json',
          body: JSON.stringify({ ...mockGroupDetail, coach_user_id: 'user-coach', coach_display_name: 'Casey Coach' }),
        });
      });
      await p.route(LIST_URL, async (route) => {
        await route.fulfill({
          status: 200,
          contentType: 'application/json',
          body: JSON.stringify({ connections: [], total: 0, viewer: 'member' }),
        });
      });
      await p.route(`**/api/groups/${GROUP_ID}/members/me/coach-consents`, async (route) => {
        if (route.request().method() === 'PUT') {
          const body = route.request().postDataJSON() as Record<string, boolean>;
          consentBodies.push(body);
          // Keeping is a fresh grant: the server stamps it, and the notice is no longer due.
          consents = { ...consents, ...body, wellness_read_at: '2026-11-06T08:00:00Z', wellness_notice_due: false };
        }
        await route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify(consents) });
      });
    });

    const block = page.getByTestId('coach-consents');
    const notice = block.getByTestId('coach-consent-wellness-notice');
    await expect(notice).toContainText('Wellness sharing now covers all your apps');
    await expect(notice).toContainText(
      'Sharing your wellness with Casey Coach now covers your sleep, HRV, resting heart rate, weight, readiness and daily notes from all your connected apps, not only Intervals.icu.',
    );
    await expect(block.getByTestId('coach-consent-wellness_read')).toBeChecked();

    // The notice holds at phone width: nothing scrolls sideways.
    await page.setViewportSize({ width: 390, height: 844 });
    expect((await measurePageLayout(page)).documentOverflowPx).toBe(0);

    await notice.getByRole('button', { name: 'Keep sharing' }).click();
    await expect.poll(() => consentBodies).toEqual([{ wellness_read: true }]);
    await expect(page.getByText('Sharing updated')).toBeVisible();
    await expect(block.getByTestId('coach-consent-wellness-notice')).toHaveCount(0);
    await expect(block.getByTestId('coach-consent-wellness_read')).toBeChecked();
  });
});

// ============================================================================
// The invite deep link
// ============================================================================

/**
 * The server's side of a held invite (carnet#1648): a hold records the code,
 * and the join answers for it once, filing the member's room, then drops it.
 * `holdStatuses` answers each hold in turn; a hold past the list is accepted.
 */
async function installInviteServer(page: Page, holdStatuses: number[] = []) {
  const state = {
    holds: [] as string[],
    joins: [] as JoinHeldInviteResponse['outcome'][],
    held: null as string | null,
  };
  // The card a link opens on reads the code once, before anything is held (carnet#1649).
  await page.route('**/api/invites/*', (route) =>
    route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({
        status: 'live',
        kind: 'member',
        inviter_name: 'Jeremy B.',
        inviter_initials: 'JB',
        has_coach: true,
        coach_name: 'Jeremy B.',
        group_name: mockGroupDetail.name,
        expires_at: null,
      }),
    }),
  );
  await page.route('**/api/user/group-invite', async (route) => {
    if (route.request().method() === 'GET') {
      // What the server holds, as the journey line and Home's strip read it.
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({
          held:
            state.held === null
              ? null
              : {
                  kind: 'member',
                  group_name: mockGroupDetail.name,
                  inviter_name: 'Jeremy B.',
                  inviter_initials: 'JB',
                  has_coach: true,
                  coach_name: 'Jeremy B.',
                  held_at: '2026-03-20T15:00:00Z',
                  state: 'pending',
                },
        }),
      });
      return;
    }
    const { code } = route.request().postDataJSON() as { code: string };
    state.holds.push(code);
    const status = holdStatuses[state.holds.length - 1] ?? 200;
    if (status !== 200) {
      await route.fulfill({ status, contentType: 'application/json', body: JSON.stringify({ error: 'unavailable' }) });
      return;
    }
    state.held = code;
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({ held: true, kind: 'member' }),
    });
  });
  await page.route('**/api/chat/groups/join', async (route) => {
    const body: JoinHeldInviteResponse =
      state.held === null
        ? { outcome: 'none' }
        : {
            outcome: 'joined',
            kind: 'member',
            group_id: GROUP_ID,
            group_name: mockGroupDetail.name,
            conversation_id: CONVERSATION_ID,
          };
    state.joins.push(body.outcome);
    state.held = null;
    await route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify(body) });
  });
  return state;
}

const carriedInvite = (page: Page) =>
  page.evaluate(() => window.sessionStorage.getItem('dravr.pending_invite_code'));

test.describe('Group invite link', () => {
  test('/groups/join/CODE hands the code to the server and opens the room it joined', async ({ page }) => {
    await setupDashboardMocks(page, { role: 'user' });
    const sent = await setupGroupMocks(page);
    const server = await installInviteServer(page);
    await loginToDashboard(page);
    await page.waitForSelector('aside', { timeout: 10000 });

    // The session's own load asked once what the server holds; the link adds nothing before its click.
    const joinsBeforeLink = [...server.joins];
    await page.goto('/groups/join/MRT2026X');

    // The card comes first, signed in or not: nothing is held or joined before the click.
    await expect(page.getByTestId('invite-card-title')).toHaveText('Jeremy B. invites you');
    expect(server.holds).toEqual([]);
    expect(server.joins).toEqual(joinsBeforeLink);
    await page.getByTestId('invite-card-join').click();
    await page.waitForSelector('aside', { timeout: 10000 });

    // The server joined: the room's conversation opens, where its notice waits.
    await expect(page).toHaveURL(new RegExp(`#chat/${CONVERSATION_ID}$`), { timeout: 10000 });
    await expect(page.getByTestId('conversation-header-title')).toHaveText(/Marathon Training 2026/);
    expect(server.holds).toEqual(['MRT2026X']);
    expect(server.joins.filter((outcome) => outcome === 'joined')).toHaveLength(1);
    expect(await carriedInvite(page)).toBeNull();
    // No `/group join` turn: the chat surface sent nothing.
    expect(sent).toEqual([]);
  });

  test('a hold the reload cut short goes out again, and the join follows it', async ({ page }) => {
    await setupDashboardMocks(page, { role: 'user' });
    await setupGroupMocks(page);
    const server = await installInviteServer(page, [503]);
    await loginToDashboard(page);
    await page.waitForSelector('aside', { timeout: 10000 });

    await page.goto('/groups/join/MRT2026X');
    await page.getByTestId('invite-card-join').click();
    await expect.poll(() => server.holds.length, { timeout: 10000 }).toBe(1);
    // The server did not take it: the card says so and stays, the tab keeps the code, and nothing joined.
    await expect(page.getByTestId('invite-card-join-failed')).toBeVisible();
    await expect.poll(() => carriedInvite(page)).toBe('MRT2026X');
    expect(server.joins).not.toContain('joined');

    // The reload a service worker update forces after sign-in.
    await page.reload();
    await page.waitForSelector('aside', { timeout: 10000 });

    await expect(page).toHaveURL(new RegExp(`#chat/${CONVERSATION_ID}$`), { timeout: 10000 });
    expect(server.holds).toEqual(['MRT2026X', 'MRT2026X']);
    expect(server.joins.filter((outcome) => outcome === 'joined')).toHaveLength(1);
    expect(await carriedInvite(page)).toBeNull();
  });
});

// ============================================================================
// Layout contract
// ============================================================================

test.describe('Group info — layout', () => {
  test('the group thread keeps its gutter for a regular user', async ({ page }) => {
    await setupDashboardMocks(page, { role: 'user' });
    await setupGroupMocks(page);
    await loginToDashboard(page);
    await page.waitForSelector('aside', { timeout: 10000 });
    await openGroups(page);

    expect(describeLayoutFailures('chat/group', await measurePageLayout(page))).toEqual([]);
  });

  test('a member row holds its role and sharing state apart on a phone', async ({ page }) => {
    // French, where the detail line is longest, so the English-labelled helper cannot open it.
    await useFrench(page);
    await setupDashboardMocks(page, { role: 'user', language: 'fr' });
    await setupGroupMocks(page, { userGroupRole: 'owner' });
    await loginToDashboard(page);
    await page.evaluate(() => {
      window.location.hash = 'chat';
    });
    await page
      .locator('[data-testid="conversation-row"]', { hasText: 'Marathon Training 2026' })
      .getByRole('button')
      .first()
      .click();
    await page.getByTestId('conversation-header-title').click();
    await page.setViewportSize({ width: 390, height: 844 });
    const panel = page.getByTestId('group-info-panel');
    await expect(panel.getByTestId('member-detail-user-2')).toBeVisible({ timeout: 10000 });
    // The owner's coach-seat block leads the panel, so the list starts below the fold.
    await panel.getByTestId('member-list').scrollIntoViewIfNeeded();
    await expectNoCollidingText(page, 'member list', '[data-testid="member-list"]', 6);
  });
});

// ============================================================================
// Group info — chat app (carnet#1683)
// ============================================================================

/**
 * A stateful `/chat-connection` mock: no chat until a code is minted, then
 * connected a read later (the bot joined), then none again once
 * disconnected. `canManage` is what the server would answer for the viewer.
 */
function chatConnectionRoutes(canManage: boolean, startConnected = false) {
  return async (page: Page) => {
    let connected = startConnected;
    let minted = false;
    let readsSinceMint = 0;
    await page.route(`**/api/groups/${GROUP_ID}/chat-connection`, async (route, request) => {
      const method = request.method();
      if (method === 'POST') {
        minted = true;
        await route.fulfill({
          status: 201,
          json: {
            channel: 'telegram',
            code: 'ABCD2345EFGH',
            deep_link_url: 'https://t.me/dravr_bot?startgroup=ABCD2345EFGH',
            command: '/group link ABCD2345EFGH',
            qr_svg: null,
            expires_at: '2099-01-01T00:10:00Z',
          },
        });
        return;
      }
      if (method === 'DELETE') {
        connected = false;
        minted = false;
        await route.fulfill({ status: 204, body: '' });
        return;
      }
      // The bot joins a while after the code is out: the read right after the
      // mint still finds no chat, the one after it finds the bot joined.
      if (minted) {
        readsSinceMint += 1;
        if (readsSinceMint >= 2) connected = true;
      }
      await route.fulfill({
        json: {
          connected,
          channel: connected ? 'telegram' : null,
          chat_title: connected ? 'Les Rouleurs chat' : null,
          connected_at: connected ? '2026-11-20T10:00:00Z' : null,
          can_manage: canManage,
        },
      });
    });
  };
}

test.describe('Group info — chat app', () => {
  test('the owner connects Telegram with the add-to-group link, then disconnects', async ({ page }) => {
    await openGroupInfo(page, {}, chatConnectionRoutes(true));
    const section = page.getByTestId('group-chat-connection');
    await expect(section.getByTestId('group-chat-not-connected')).toBeVisible();

    await section.getByTestId('group-chat-connect').click();
    await section.getByTestId('group-chat-choose-telegram').click();

    const open = section.getByTestId('group-chat-open-app');
    await expect(open).toHaveAttribute('href', 'https://t.me/dravr_bot?startgroup=ABCD2345EFGH');
    await expect(section.getByTestId('group-chat-command')).toHaveText('/group link ABCD2345EFGH');

    // The panel reads the connection again until the bot has joined.
    await expect(section.getByTestId('group-chat-title')).toHaveText('Les Rouleurs chat', {
      timeout: 10000,
    });
    await expect(section.getByTestId('group-chat-connected-to')).toHaveText('Connected to Telegram');

    await section.getByTestId('group-chat-disconnect').click();
    // Group info is a dialog too; the confirm is the one titled for the chat.
    const confirm = page.getByRole('dialog', { name: 'Disconnect Telegram?' });
    await confirm.getByRole('button', { name: 'Disconnect' }).click();
    await expect(section.getByTestId('group-chat-not-connected')).toBeVisible({ timeout: 10000 });
  });

  test('a member reads which chat the group is in, and cannot change it', async ({ page }) => {
    await openGroupInfo(page, { userGroupRole: 'member' }, chatConnectionRoutes(false, true));
    const section = page.getByTestId('group-chat-connection');
    await expect(section.getByTestId('group-chat-title')).toHaveText('Les Rouleurs chat');
    await expect(section.getByTestId('group-chat-disconnect')).toHaveCount(0);
    await expect(section.getByTestId('group-chat-connect')).toHaveCount(0);
  });
});
