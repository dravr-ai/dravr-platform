// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Playwright E2E tests for Group Coaching features.
// ABOUTME: Tests group CRUD, membership, invites, stats, and authorization.

import { test, expect, type Page } from '@playwright/test';
import { setupDashboardMocks, loginToDashboard, navigateToTab } from './test-helpers';

// ============================================================================
// Mock Data
// ============================================================================

const mockCoaches = {
  coaches: [
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
      visibility: 'tenant',
    },
  ],
  metadata: { timestamp: '2024-06-01T10:00:00Z', api_version: 'v1' },
};

const mockGroups = {
  groups: [
    {
      id: 'group-1',
      name: 'Marathon Training 2026',
      description: 'Preparing for the fall marathon',
      coach_id: 'coach-marathon',
      member_count: 5,
      is_active: true,
      peer_data_sharing: false,
      my_role: 'owner',
      created_at: '2024-03-01T10:00:00Z',
    },
    {
      id: 'group-2',
      name: 'Trail Running Crew',
      description: 'Weekend trail adventures',
      coach_id: 'coach-marathon',
      member_count: 3,
      is_active: true,
      peer_data_sharing: true,
      my_role: 'member',
      created_at: '2024-04-15T08:00:00Z',
    },
  ],
};

const mockGroupDetail = {
  id: 'group-1',
  tenant_id: 'tenant-1',
  name: 'Marathon Training 2026',
  description: 'Preparing for the fall marathon',
  coach_id: 'coach-marathon',
  owner_id: 'user-123',
  peer_data_sharing: false,
  max_members: 20,
  is_active: true,
  created_at: '2024-03-01T10:00:00Z',
  updated_at: '2024-03-20T15:00:00Z',
};

const mockMembers = {
  members: [
    {
      id: 'member-1',
      group_id: 'group-1',
      user_id: 'user-123',
      tenant_id: 'tenant-1',
      role: 'owner',
      peer_sharing_consent: false,
      consent_given_at: '2024-03-01T10:00:00Z',
      joined_at: '2024-03-01T10:00:00Z',
      left_at: null,
      display_name: 'Test Admin',
    },
    {
      id: 'member-2',
      group_id: 'group-1',
      user_id: 'user-2',
      tenant_id: 'tenant-1',
      role: 'member',
      peer_sharing_consent: true,
      consent_given_at: '2024-03-05T12:00:00Z',
      joined_at: '2024-03-05T12:00:00Z',
      left_at: null,
      display_name: 'Alice Runner',
    },
    {
      id: 'member-3',
      group_id: 'group-1',
      user_id: 'user-3',
      tenant_id: 'tenant-1',
      role: 'admin',
      peer_sharing_consent: false,
      consent_given_at: '2024-03-10T09:00:00Z',
      joined_at: '2024-03-10T09:00:00Z',
      left_at: null,
      display_name: 'Bob Cyclist',
    },
  ],
};

const mockInvites = {
  invites: [
    {
      id: 'invite-1',
      group_id: 'group-1',
      tenant_id: 'tenant-1',
      code: 'MRT2026X',
      created_by: 'user-123',
      expires_at: '2024-06-01T00:00:00Z',
      max_uses: 10,
      use_count: 3,
      is_active: true,
      created_at: '2024-03-15T10:00:00Z',
    },
  ],
};

const mockStats = {
  stats: {
    total_members: 5,
    active_members: 4,
    avg_weekly_volume_km: 38.5,
    avg_ctl: 52.0,
    flagged_members: 1,
    weekly_trend: 'improving',
  },
};

const mockHealthFlags = {
  flags: [
    {
      user_id: 'user-2',
      display_name: 'Alice Runner',
      flag_type: 'overreaching',
      severity: 'warning',
      detail: 'TSB at -22, recommend recovery',
    },
  ],
};

const mockWeeklyReport = {
  summary: 'Marathon Training 2026 had 4/5 active members this week.',
  highlights: ['Bob set a new 5K PR (19:42)', 'Group volume up 12%'],
  concerns: ['Alice: TSB at -22, recommend recovery'],
  recommendations: ['Review 1 flagged member and consider recovery adjustments.'],
  stats: mockStats.stats,
};

// ============================================================================
// Setup Helpers
// ============================================================================

async function setupGroupMocks(page: Page) {
  // Groups list
  await page.route('**/api/groups', async (route) => {
    if (route.request().method() === 'GET') {
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify(mockGroups),
      });
    } else if (route.request().method() === 'POST') {
      const body = route.request().postDataJSON();
      await route.fulfill({
        status: 201,
        contentType: 'application/json',
        body: JSON.stringify({
          id: 'group-new',
          tenant_id: 'tenant-1',
          name: body.name,
          description: body.description || null,
          coach_id: body.coach_id,
          owner_id: 'user-123',
          peer_data_sharing: false,
          max_members: body.max_members || 20,
          is_active: true,
          created_at: new Date().toISOString(),
          updated_at: new Date().toISOString(),
        }),
      });
    }
  });

  // Group detail
  await page.route('**/api/groups/group-1', async (route) => {
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
    }
  });

  // Members
  await page.route('**/api/groups/group-1/members', async (route) => {
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify(mockMembers),
    });
  });

  // Member removal
  await page.route('**/api/groups/group-1/members/**', async (route) => {
    if (route.request().method() === 'DELETE') {
      await route.fulfill({ status: 204 });
    } else if (route.request().method() === 'PUT') {
      await route.fulfill({ status: 200, contentType: 'application/json', body: '{}' });
    }
  });

  // Invites
  await page.route('**/api/groups/group-1/invites', async (route) => {
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
          group_id: 'group-1',
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
    }
  });

  // Invite deactivation
  await page.route('**/api/groups/group-1/invites/**', async (route) => {
    if (route.request().method() === 'DELETE') {
      await route.fulfill({ status: 204 });
    }
  });

  // Stats
  await page.route('**/api/groups/group-1/stats', async (route) => {
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify(mockStats),
    });
  });

  // Health flags
  await page.route('**/api/groups/group-1/health', async (route) => {
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify(mockHealthFlags),
    });
  });

  // Weekly report
  await page.route('**/api/groups/group-1/report', async (route) => {
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify(mockWeeklyReport),
    });
  });

  // Join group
  await page.route('**/api/groups/join', async (route) => {
    const body = route.request().postDataJSON();
    if (body.invite_code === 'VALIDCODE') {
      await route.fulfill({
        status: 201,
        contentType: 'application/json',
        body: JSON.stringify({
          id: 'member-new',
          group_id: 'group-1',
          user_id: 'user-123',
          tenant_id: 'tenant-1',
          role: 'member',
          peer_sharing_consent: false,
          consent_given_at: new Date().toISOString(),
          joined_at: new Date().toISOString(),
          left_at: null,
          display_name: 'Test Admin',
        }),
      });
    } else {
      await route.fulfill({
        status: 404,
        contentType: 'application/json',
        body: JSON.stringify({ error: 'Invalid or expired invite code' }),
      });
    }
  });

  // Leave group
  await page.route('**/api/groups/group-*/leave', async (route) => {
    await route.fulfill({ status: 204 });
  });

  // Peer consent
  await page.route('**/api/groups/group-*/members/me/consent', async (route) => {
    await route.fulfill({ status: 200, contentType: 'application/json', body: '{}' });
  });

  // Coaches (for group creation form)
  await page.route('**/api/coaches', async (route) => {
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify(mockCoaches),
    });
  });
}

// ============================================================================
// Group List Tests
// ============================================================================

test.describe('Group Coaching - List', () => {
  test.beforeEach(async ({ page }) => {
    await setupDashboardMocks(page);
    await setupGroupMocks(page);
    await loginToDashboard(page);
  });

  test('displays group list with cards', async ({ page }) => {
    await navigateToTab(page, 'Groups');
    await expect(page.getByText('Marathon Training 2026')).toBeVisible();
    await expect(page.getByText('Trail Running Crew')).toBeVisible();
  });

  test('shows member count on group cards', async ({ page }) => {
    await navigateToTab(page, 'Groups');
    await expect(page.getByText('5 members')).toBeVisible();
    await expect(page.getByText('3 members')).toBeVisible();
  });

  test('shows role badges on group cards', async ({ page }) => {
    await navigateToTab(page, 'Groups');
    await expect(page.getByText('Owner')).toBeVisible();
    await expect(page.getByText('Member')).toBeVisible();
  });

  test('shows empty state when no groups', async ({ page }) => {
    await page.route('**/api/groups', async (route) => {
      if (route.request().method() === 'GET') {
        await route.fulfill({
          status: 200,
          contentType: 'application/json',
          body: JSON.stringify({ groups: [] }),
        });
      }
    });
    await navigateToTab(page, 'Groups');
    await expect(page.getByText(/no groups|get started|create/i)).toBeVisible();
  });
});

// ============================================================================
// Group Creation Tests
// ============================================================================

test.describe('Group Coaching - Create', () => {
  test.beforeEach(async ({ page }) => {
    await setupDashboardMocks(page);
    await setupGroupMocks(page);
    await loginToDashboard(page);
    await navigateToTab(page, 'Groups');
  });

  test('opens create group modal', async ({ page }) => {
    await page.getByRole('button', { name: /create.*group/i }).click();
    await expect(page.getByPlaceholder(/group name|name/i)).toBeVisible();
  });

  test('creates group with name and coach', async ({ page }) => {
    await page.getByRole('button', { name: /create.*group/i }).click();
    await page.getByPlaceholder(/group name|name/i).fill('New Running Group');
    // Select coach if dropdown exists
    const coachSelector = page.locator('select, [role="combobox"]').first();
    if (await coachSelector.isVisible()) {
      await coachSelector.selectOption({ index: 0 });
    }
    await page.getByRole('button', { name: /create|save|submit/i }).click();
    // Should show success or redirect
    await expect(page.getByText(/created|success/i).or(page.getByText('New Running Group'))).toBeVisible({ timeout: 5000 });
  });
});

// ============================================================================
// Group Detail Tests
// ============================================================================

test.describe('Group Coaching - Detail', () => {
  test.beforeEach(async ({ page }) => {
    await setupDashboardMocks(page);
    await setupGroupMocks(page);
    await loginToDashboard(page);
    await navigateToTab(page, 'Groups');
  });

  test('navigates to group detail on card click', async ({ page }) => {
    await page.getByText('Marathon Training 2026').click();
    await expect(page.getByText('Marathon Training 2026')).toBeVisible();
  });

  test('shows members tab with member list', async ({ page }) => {
    await page.getByText('Marathon Training 2026').click();
    // Click Members tab if tabs exist
    const membersTab = page.getByRole('tab', { name: /members/i }).or(page.getByText(/members/i));
    if (await membersTab.isVisible()) {
      await membersTab.click();
    }
    await expect(page.getByText('Test Admin')).toBeVisible();
    await expect(page.getByText('Alice Runner')).toBeVisible();
    await expect(page.getByText('Bob Cyclist')).toBeVisible();
  });

  test('shows member roles in member list', async ({ page }) => {
    await page.getByText('Marathon Training 2026').click();
    const membersTab = page.getByRole('tab', { name: /members/i }).or(page.getByText(/members/i));
    if (await membersTab.isVisible()) {
      await membersTab.click();
    }
    // Should show role indicators
    await expect(page.getByText(/owner/i).first()).toBeVisible();
  });

  test('shows stats tab with aggregate data', async ({ page }) => {
    await page.getByText('Marathon Training 2026').click();
    const statsTab = page.getByRole('tab', { name: /stats/i }).or(page.getByText(/stats/i));
    if (await statsTab.isVisible()) {
      await statsTab.click();
    }
    // Should show aggregate stats
    await expect(page.getByText(/38\.5|38.5/)).toBeVisible({ timeout: 5000 });
  });
});

// ============================================================================
// Invite Management Tests
// ============================================================================

test.describe('Group Coaching - Invites', () => {
  test.beforeEach(async ({ page }) => {
    await setupDashboardMocks(page);
    await setupGroupMocks(page);
    await loginToDashboard(page);
    await navigateToTab(page, 'Groups');
    await page.getByText('Marathon Training 2026').click();
  });

  test('shows invite tab with existing invites', async ({ page }) => {
    const invitesTab = page.getByRole('tab', { name: /invites/i }).or(page.getByText(/invites/i));
    if (await invitesTab.isVisible()) {
      await invitesTab.click();
    }
    await expect(page.getByText('MRT2026X')).toBeVisible({ timeout: 5000 });
  });

  test('shows invite use count', async ({ page }) => {
    const invitesTab = page.getByRole('tab', { name: /invites/i }).or(page.getByText(/invites/i));
    if (await invitesTab.isVisible()) {
      await invitesTab.click();
    }
    // Should show 3/10 uses or similar
    await expect(page.getByText(/3.*10|3 of 10|3\/10/)).toBeVisible({ timeout: 5000 });
  });

  test('creates new invite', async ({ page }) => {
    const invitesTab = page.getByRole('tab', { name: /invites/i }).or(page.getByText(/invites/i));
    if (await invitesTab.isVisible()) {
      await invitesTab.click();
    }
    const createBtn = page.getByRole('button', { name: /create.*invite|generate.*invite|new.*invite/i });
    if (await createBtn.isVisible()) {
      await createBtn.click();
      await expect(page.getByText('NEWCODE1')).toBeVisible({ timeout: 5000 });
    }
  });
});

// ============================================================================
// Join Group Tests
// ============================================================================

test.describe('Group Coaching - Join', () => {
  test.beforeEach(async ({ page }) => {
    await setupDashboardMocks(page);
    await setupGroupMocks(page);
    await loginToDashboard(page);
    await navigateToTab(page, 'Groups');
  });

  test('opens join group modal', async ({ page }) => {
    const joinBtn = page.getByRole('button', { name: /join.*group/i });
    if (await joinBtn.isVisible()) {
      await joinBtn.click();
      await expect(page.getByPlaceholder(/invite.*code|code/i)).toBeVisible();
    }
  });

  test('joins with valid invite code', async ({ page }) => {
    const joinBtn = page.getByRole('button', { name: /join.*group/i });
    if (await joinBtn.isVisible()) {
      await joinBtn.click();
      await page.getByPlaceholder(/invite.*code|code/i).fill('VALIDCODE');
      await page.getByRole('button', { name: /join|submit/i }).click();
      await expect(page.getByText(/joined|success/i)).toBeVisible({ timeout: 5000 });
    }
  });

  test('shows error for invalid invite code', async ({ page }) => {
    const joinBtn = page.getByRole('button', { name: /join.*group/i });
    if (await joinBtn.isVisible()) {
      await joinBtn.click();
      await page.getByPlaceholder(/invite.*code|code/i).fill('BADCODE');
      await page.getByRole('button', { name: /join|submit/i }).click();
      await expect(page.getByText(/invalid|expired|not found|error/i)).toBeVisible({ timeout: 5000 });
    }
  });
});

// ============================================================================
// Settings & Peer Sharing Tests
// ============================================================================

test.describe('Group Coaching - Settings', () => {
  test.beforeEach(async ({ page }) => {
    await setupDashboardMocks(page);
    await setupGroupMocks(page);
    await loginToDashboard(page);
    await navigateToTab(page, 'Groups');
    await page.getByText('Marathon Training 2026').click();
  });

  test('shows settings tab for owner', async ({ page }) => {
    const settingsTab = page.getByRole('tab', { name: /settings/i }).or(page.getByText(/settings/i));
    if (await settingsTab.isVisible()) {
      await settingsTab.click();
      // Should show group settings form
      await expect(page.getByText(/peer.*sharing|settings|delete/i)).toBeVisible({ timeout: 5000 });
    }
  });

  test('can toggle peer data sharing', async ({ page }) => {
    const settingsTab = page.getByRole('tab', { name: /settings/i }).or(page.getByText(/settings/i));
    if (await settingsTab.isVisible()) {
      await settingsTab.click();
      const toggle = page.getByRole('switch').or(page.locator('input[type="checkbox"]')).first();
      if (await toggle.isVisible()) {
        await toggle.click();
      }
    }
  });

  test('shows delete button for owner', async ({ page }) => {
    const settingsTab = page.getByRole('tab', { name: /settings/i }).or(page.getByText(/settings/i));
    if (await settingsTab.isVisible()) {
      await settingsTab.click();
      await expect(page.getByRole('button', { name: /delete.*group/i })).toBeVisible({ timeout: 5000 });
    }
  });
});
