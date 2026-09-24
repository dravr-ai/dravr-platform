// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: E2E for group management after the Groups tab: the group row, its header, and Group info
// ABOUTME: Roster, invites, settings, consent, the digest gate, the exits, TrainingPeaks links, the /groups/join landing

import { test, expect, type Page } from '@playwright/test';
import type {
  GroupHealthFlag,
  GroupStatsResponse,
  GroupWeeklyReportResponse,
} from '@pierre/shared-types';
import { setupDashboardMocks, loginToDashboard } from './test-helpers';
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
  coach_user_id: null,
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
      created_at: '2026-03-15T10:00:00Z',
    },
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
};

// ============================================================================
// Setup Helpers
// ============================================================================

interface GroupMockOptions {
  userGroupRole?: 'owner' | 'admin' | 'member';
  /** The tenant tier flag gating the weekly report + health flags panel. */
  weeklyDigest?: boolean;
}

/** Every turn the chat surface sent, in order. */
interface SentTurn {
  conversationId: string;
  content: string;
}

async function setupGroupMocks(page: Page, options: GroupMockOptions = {}): Promise<SentTurn[]> {
  const { userGroupRole = 'owner', weeklyDigest = true } = options;
  const mockMembers = buildMockMembers(userGroupRole);
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
        body: JSON.stringify(mockGroupDetail),
      });
    } else if (route.request().method() === 'PUT') {
      const body = route.request().postDataJSON();
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({ ...mockGroupDetail, ...body }),
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
      body: JSON.stringify({ flags: mockHealthFlags, total: mockHealthFlags.length }),
    });
  });

  await page.route(`**/api/groups/${GROUP_ID}/report`, async (route) => {
    const body: GroupWeeklyReportResponse = {
      report: {
        stats: mockStats.stats,
        fresh_members: [{ user_id: 'user-456', display_name: 'Alice Runner', form_pct: 12, tsb: 9 }],
      },
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
  await setupDashboardMocks(page, { role: 'user' });
  const sent = await setupGroupMocks(page, options);
  if (extraRoutes) await extraRoutes(page);
  await loginToDashboard(page);
  await page.waitForSelector('aside', { timeout: 10000 });

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

  test('lists every member with their role', async ({ page }) => {
    await openGroupInfo(page);

    const roster = page.getByRole('table');
    await expect(roster.getByText('Test User')).toBeVisible();
    await expect(roster.getByText('Alice Runner')).toBeVisible();
    await expect(roster.getByText('Bob Cyclist')).toBeVisible();
    await expect(roster.getByText('Owner')).toBeVisible();
    await expect(roster.getByText('Admin')).toBeVisible();
  });

  test('an owner can promote and remove a member', async ({ page }) => {
    await openGroupInfo(page, { userGroupRole: 'owner' });

    await page.getByLabel(/Promote Alice Runner/).click();
    await expect(page.getByText(/Role updated/i)).toBeVisible({ timeout: 5000 });

    await page.getByLabel(/Remove Alice Runner/).click();
    await expect(page.getByText('Remove Member')).toBeVisible();
    await page.getByRole('button', { name: 'Remove' }).last().click();
    await expect(page.getByText('Member removed')).toBeVisible({ timeout: 5000 });
  });

  test('a plain member gets no roster actions and no settings form', async ({ page }) => {
    await openGroupInfo(page, { userGroupRole: 'member' });

    await expect(page.getByText('Actions')).toHaveCount(0);
    await expect(page.getByTestId('group-info-save-settings')).toHaveCount(0);
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

    await page.getByRole('button', { name: 'New Invite' }).click();
    await expect(page.getByText('Create Invite Link')).toBeVisible();
    await page.getByRole('button', { name: 'Create' }).click();
    await expect(page.getByText('Invite created')).toBeVisible({ timeout: 5000 });
  });
});

// ============================================================================
// Group info — settings, consent, analytics
// ============================================================================

test.describe('Group info — settings and consent', () => {
  test('an admin saves the group settings', async ({ page }) => {
    await openGroupInfo(page, { userGroupRole: 'owner' });

    const nameInput = page.getByLabel('Group Name', { exact: true });
    await nameInput.clear();
    await nameInput.fill('Renamed Group');
    await page.getByTestId('group-info-save-settings').click();
    await expect(page.getByText('Settings saved')).toBeVisible({ timeout: 5000 });
  });

  test('binds the peer-consent switch to the caller own membership row', async ({ page }) => {
    const consentBodies: Array<Record<string, unknown>> = [];
    await openGroupInfo(page);
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

  test('shows the weekly report and one row per flagged member', async ({ page }) => {
    await openGroupInfo(page);

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
      'Members at high overtraining risk: 1. Consider adjusting their recovery.',
      'Group volume is up on last week — make sure recovery keeps pace.',
    ]);
    await expect(page.getByTestId('group-health-flag-row')).toHaveCount(2);
    await expect(page.getByTestId('group-health-flag-row').nth(1)).toContainText('No activity for 11 days');
    await expect(page.getByText('Health flags (2)')).toBeVisible();
  });

  test('withholds the weekly report when the tenant tier does not include it', async ({ page }) => {
    await openGroupInfo(page, { weeklyDigest: false });

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
    await page.getByTestId('group-info-leave').click();
    await expect(page.getByText('You will need a new invite to rejoin')).toBeVisible();
    await page.getByRole('button', { name: 'Leave Group' }).last().click();
    await expect(page.getByText('Left group')).toBeVisible({ timeout: 5000 });
  });

  test('an owner archives the group', async ({ page }) => {
    await openGroupInfo(page, { userGroupRole: 'owner' });

    await expect(page.getByTestId('group-info-leave')).toHaveCount(0);
    await page.getByTestId('group-info-delete').click();
    await expect(page.getByText('This will permanently archive')).toBeVisible();
    await page.getByRole('button', { name: 'Delete Group' }).last().click();
    await expect(page.getByText('Group deleted')).toBeVisible({ timeout: 5000 });
  });
});

// ============================================================================
// Group info — TrainingPeaks links through the group's coach
// ============================================================================

/** A link as the group's delegated-connection routes serialise it. */
function delegatedLink(status: 'proposed' | 'confirmed', memberUserId: string, memberName: string) {
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
    metadata: { timestamp: '2026-09-24T09:00:00Z', api_version: '1.0' },
  };
}

const LIST_URL = `**/api/groups/${GROUP_ID}/delegated-connections`;

test.describe('Group info — TrainingPeaks links', () => {
  test('the coach links a roster athlete to a member and waits for them', async ({ page }) => {
    const proposals: Array<Record<string, unknown>> = [];
    let linked = false;

    await openGroupInfo(page, {}, async (p) => {
      // The signed-in user is the group's human coach and holds no membership.
      await p.route(`**/api/groups/${GROUP_ID}`, async (route) => {
        await route.fulfill({
          status: 200,
          contentType: 'application/json',
          body: JSON.stringify({ ...mockGroupDetail, coach_user_id: 'user-123', owner_id: 'user-3' }),
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
    // A coach holds no membership: nothing to leave, archive or consent to, and
    // of the settings only where the weekly digest goes.
    await expect(page.getByTestId('group-info-leave')).toHaveCount(0);
    await expect(page.getByTestId('group-info-delete')).toHaveCount(0);
    await expect(page.getByTestId('peer-consent-card')).toHaveCount(0);
    await expect(page.getByTestId('group-info-remove-coach')).toHaveCount(0);
    await expect(page.getByTestId('group-digest-mode')).toBeVisible();
    await expect(page.getByLabel('Group Name')).toHaveCount(0);

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
          body: JSON.stringify({ ...mockGroupDetail, coach_user_id: 'user-coach' }),
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
});

// ============================================================================
// The invite deep link
// ============================================================================

test.describe('Group invite link', () => {
  test('/groups/join/CODE lands on chat and sends /group join CODE', async ({ page }) => {
    await setupDashboardMocks(page, { role: 'user' });
    const sent = await setupGroupMocks(page);
    await loginToDashboard(page);
    await page.waitForSelector('aside', { timeout: 10000 });

    await page.goto('/groups/join/MRT2026X');
    await page.waitForSelector('aside', { timeout: 10000 });

    // The Groups tab is gone: the link resolves to chat and joins by command.
    await expect(page).toHaveURL(/#chat/);
    await expect.poll(() => sent.length, { timeout: 10000 }).toBe(1);
    expect(sent[0].content).toBe('/group join MRT2026X');
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

    expect(describeLayoutFailures('chat/group', await measurePageLayout(page))).toEqual([]);
  });
});
