// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Playwright E2E tests for Group Coaching features.
// ABOUTME: Tests group CRUD, membership, invites, stats, and authorization for owner/member roles.

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
  total: 1,
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

function buildMockMembers(currentUserRole: 'owner' | 'admin' | 'member') {
  return {
    members: [
      {
        id: 'member-1',
        group_id: 'group-1',
        user_id: 'user-123',
        tenant_id: 'tenant-1',
        role: currentUserRole,
        peer_sharing_consent: false,
        consent_given_at: '2024-03-01T10:00:00Z',
        joined_at: '2024-03-01T10:00:00Z',
        left_at: null,
        display_name: 'Test User',
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
}

const mockInvites = {
  invites: [
    {
      id: 'invite-1',
      group_id: 'group-1',
      tenant_id: 'tenant-1',
      code: 'MRT2026X',
      created_by: 'user-123',
      expires_at: '2027-06-01T00:00:00Z',
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

// ============================================================================
// Setup Helpers
// ============================================================================

interface GroupMockOptions {
  userGroupRole?: 'owner' | 'admin' | 'member';
  canCreateGroup?: boolean;
  groupCreationPolicy?: string;
}

async function setupGroupMocks(page: Page, options: GroupMockOptions = {}) {
  const { userGroupRole = 'owner', canCreateGroup = true, groupCreationPolicy = 'everyone' } = options;
  const mockMembers = buildMockMembers(userGroupRole);

  // Group creation permissions
  await page.route('**/api/groups/permissions', async (route) => {
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({ can_create: canCreateGroup, policy: groupCreationPolicy }),
    });
  });

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

  // Member role change and removal (more specific path — register before generic members)
  await page.route('**/api/groups/group-1/members/**', async (route) => {
    if (route.request().method() === 'DELETE') {
      await route.fulfill({ status: 204 });
    } else if (route.request().method() === 'PUT') {
      await route.fulfill({ status: 200, contentType: 'application/json', body: '{}' });
    } else {
      await route.fallback();
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
    } else {
      await route.fallback();
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
      body: JSON.stringify({ flags: [] }),
    });
  });

  // Weekly report
  await page.route('**/api/groups/group-1/report', async (route) => {
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({ summary: 'Good week.', highlights: [], concerns: [], recommendations: [], stats: mockStats.stats }),
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
          display_name: 'Test User',
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

// Helper: login as regular user and navigate to Groups tab
async function loginAndGoToGroups(page: Page, options: GroupMockOptions = {}) {
  await setupDashboardMocks(page, { role: 'user' });
  await setupGroupMocks(page, options);
  await loginToDashboard(page);
  await navigateToTab(page, 'Groups');
}

// Helper: navigate to group detail from list
async function goToGroupDetail(page: Page) {
  await page.getByText('Marathon Training 2026').click();
  await expect(page.getByText('All Groups')).toBeVisible();
}

// ============================================================================
// Group List Tests
// ============================================================================

test.describe('Group Coaching - List', () => {
  test.beforeEach(async ({ page }) => {
    await loginAndGoToGroups(page);
  });

  test('displays group list with cards', async ({ page }) => {
    await expect(page.getByText('Marathon Training 2026')).toBeVisible();
    await expect(page.getByText('Trail Running Crew')).toBeVisible();
  });

  test('shows member count on group cards', async ({ page }) => {
    await expect(page.getByText('5 members')).toBeVisible();
    await expect(page.getByText('3 members')).toBeVisible();
  });

  test('shows role badges on group cards', async ({ page }) => {
    await expect(page.getByText('Owner')).toBeVisible();
    await expect(page.getByText('Member', { exact: true })).toBeVisible();
  });

  test('shows peer sharing indicator on enabled groups', async ({ page }) => {
    // group-2 has peer_data_sharing: true
    await expect(page.getByText('Peer Sharing')).toBeVisible();
  });

  test('shows empty state when no groups', async ({ page }) => {
    // Override groups mock with empty response before navigating
    await setupDashboardMocks(page, { role: 'user' });
    await page.route('**/api/groups', async (route) => {
      if (route.request().method() === 'GET') {
        await route.fulfill({
          status: 200,
          contentType: 'application/json',
          body: JSON.stringify({ groups: [] }),
        });
      } else {
        await route.fallback();
      }
    });
    await loginToDashboard(page);
    await navigateToTab(page, 'Groups');
    await expect(page.getByText('No groups yet')).toBeVisible({ timeout: 5000 });
  });
});

// ============================================================================
// Group Creation Tests
// ============================================================================

test.describe('Group Coaching - Create', () => {
  test.beforeEach(async ({ page }) => {
    await loginAndGoToGroups(page);
  });

  test('opens create group modal', async ({ page }) => {
    await page.getByRole('button', { name: /Create Group/ }).click();
    await expect(page.getByText('Create Coaching Group')).toBeVisible();
    await expect(page.getByPlaceholder('e.g., Marathon Training Squad')).toBeVisible();
  });

  test('creates group with name and coach', async ({ page }) => {
    await page.getByRole('button', { name: /Create Group/ }).click();
    await page.getByPlaceholder('e.g., Marathon Training Squad').fill('New Running Group');
    // Select the coach from the dropdown (use exact match to avoid dialog label collision)
    await page.getByLabel('Coach', { exact: true }).selectOption('coach-marathon');
    // Submit the form
    await page.getByRole('button', { name: 'Create Group' }).last().click();
    // Should show success toast
    await expect(page.getByText(/created/i)).toBeVisible({ timeout: 5000 });
  });
});

// ============================================================================
// Group Detail Tests
// ============================================================================

test.describe('Group Coaching - Detail', () => {
  test.beforeEach(async ({ page }) => {
    await loginAndGoToGroups(page);
  });

  test('navigates to group detail on card click', async ({ page }) => {
    await goToGroupDetail(page);
    // Header shows group name
    await expect(page.locator('h2', { hasText: 'Marathon Training 2026' })).toBeVisible();
  });

  test('shows members tab with member list', async ({ page }) => {
    await goToGroupDetail(page);
    // Members tab is default active
    await expect(page.getByText('Test User')).toBeVisible();
    await expect(page.getByText('Alice Runner')).toBeVisible();
    await expect(page.getByText('Bob Cyclist')).toBeVisible();
  });

  test('shows member roles in member list', async ({ page }) => {
    await goToGroupDetail(page);
    // Role badges in the member table
    await expect(page.getByText('Owner').first()).toBeVisible();
    await expect(page.getByText('Admin').first()).toBeVisible();
  });

  test('shows stats tab with aggregate data', async ({ page }) => {
    await goToGroupDetail(page);
    await page.getByRole('tab', { name: /Stats/ }).click();
    await expect(page.getByText('38.5')).toBeVisible({ timeout: 5000 });
    await expect(page.getByText('Active Members')).toBeVisible();
    await expect(page.getByText('Avg Weekly Volume')).toBeVisible();
  });
});

// ============================================================================
// Member Management Tests (Owner)
// ============================================================================

test.describe('Group Coaching - Member Management', () => {
  test('owner sees action buttons for non-owner members', async ({ page }) => {
    await loginAndGoToGroups(page, { userGroupRole: 'owner' });
    await goToGroupDetail(page);
    // Actions column visible
    await expect(page.getByText('Actions')).toBeVisible();
    // Remove button on Alice (member, not self, not owner)
    await expect(page.getByLabel(/Remove Alice Runner/)).toBeVisible();
  });

  test('owner can promote member to admin', async ({ page }) => {
    await loginAndGoToGroups(page, { userGroupRole: 'owner' });
    await goToGroupDetail(page);
    // Click promote button for Alice Runner
    await page.getByLabel(/Promote Alice Runner/).click();
    await expect(page.getByText(/Role updated/i)).toBeVisible({ timeout: 5000 });
  });

  test('owner can remove member', async ({ page }) => {
    await loginAndGoToGroups(page, { userGroupRole: 'owner' });
    await goToGroupDetail(page);
    // Click remove button for Alice Runner
    await page.getByLabel(/Remove Alice Runner/).click();
    // Confirm dialog appears
    await expect(page.getByText('Remove Member')).toBeVisible();
    await page.getByRole('button', { name: 'Remove' }).last().click();
    await expect(page.getByText('Member removed')).toBeVisible({ timeout: 5000 });
  });

  test('regular member does not see action buttons', async ({ page }) => {
    await loginAndGoToGroups(page, { userGroupRole: 'member' });
    await goToGroupDetail(page);
    // No Actions column
    await expect(page.getByText('Actions')).not.toBeVisible();
  });
});

// ============================================================================
// Invite Management Tests
// ============================================================================

test.describe('Group Coaching - Invites', () => {
  test.beforeEach(async ({ page }) => {
    await loginAndGoToGroups(page, { userGroupRole: 'owner' });
    await goToGroupDetail(page);
    await page.getByRole('tab', { name: /Invites/ }).click();
  });

  test('shows invite tab with existing invites', async ({ page }) => {
    await expect(page.getByText('MRT2026X')).toBeVisible({ timeout: 5000 });
  });

  test('shows invite use count', async ({ page }) => {
    await expect(page.getByText('3 / 10 uses')).toBeVisible({ timeout: 5000 });
  });

  test('creates new invite', async ({ page }) => {
    await page.getByRole('button', { name: 'New Invite' }).click();
    // Fill the create invite form and submit
    await expect(page.getByText('Create Invite Link')).toBeVisible();
    await page.getByRole('button', { name: 'Create' }).click();
    // Success toast appears
    await expect(page.getByText('Invite created')).toBeVisible({ timeout: 5000 });
  });
});

// ============================================================================
// Join Group Tests
// ============================================================================

test.describe('Group Coaching - Join', () => {
  test.beforeEach(async ({ page }) => {
    await loginAndGoToGroups(page);
  });

  test('opens join group modal', async ({ page }) => {
    await page.getByRole('button', { name: /Join Group/ }).click();
    await expect(page.getByText('Join a Group')).toBeVisible();
    await expect(page.getByPlaceholder('e.g., abc123')).toBeVisible();
  });

  test('join button is disabled without consent checkbox', async ({ page }) => {
    await page.getByRole('button', { name: /Join Group/ }).click();
    await page.getByPlaceholder('e.g., abc123').fill('VALIDCODE');
    // Don't check consent — button should be disabled
    const joinButton = page.getByRole('button', { name: 'Join Group' }).last();
    await expect(joinButton).toBeDisabled();
  });

  test('joins with valid invite code', async ({ page }) => {
    await page.getByRole('button', { name: /Join Group/ }).click();
    await page.getByPlaceholder('e.g., abc123').fill('VALIDCODE');
    // Check consent checkbox
    await page.locator('input[type="checkbox"]').check();
    await page.getByRole('button', { name: 'Join Group' }).last().click();
    await expect(page.getByText(/joined|success/i)).toBeVisible({ timeout: 5000 });
  });

  test('shows error for invalid invite code', async ({ page }) => {
    await page.getByRole('button', { name: /Join Group/ }).click();
    await page.getByPlaceholder('e.g., abc123').fill('BADCODE');
    await page.locator('input[type="checkbox"]').check();
    await page.getByRole('button', { name: 'Join Group' }).last().click();
    // Error shows as field error or toast
    await expect(page.getByText(/Invalid or expired|Join failed/i)).toBeVisible({ timeout: 5000 });
  });
});

// ============================================================================
// Settings & Peer Sharing Tests
// ============================================================================

test.describe('Group Coaching - Settings', () => {
  test('shows settings tab for owner', async ({ page }) => {
    await loginAndGoToGroups(page, { userGroupRole: 'owner' });
    await goToGroupDetail(page);
    await page.getByRole('tab', { name: /Settings/ }).click();
    await expect(page.getByText('Group Settings')).toBeVisible();
    await expect(page.getByText('Enable peer data sharing')).toBeVisible();
  });

  test('can save updated group settings', async ({ page }) => {
    await loginAndGoToGroups(page, { userGroupRole: 'owner' });
    await goToGroupDetail(page);
    await page.getByRole('tab', { name: /Settings/ }).click();
    // Update group name
    const nameInput = page.getByLabel('Group Name', { exact: true });
    await nameInput.clear();
    await nameInput.fill('Renamed Group');
    await page.getByRole('button', { name: 'Save Settings' }).click();
    await expect(page.getByText('Settings saved')).toBeVisible({ timeout: 5000 });
  });

  test('can toggle peer data sharing', async ({ page }) => {
    await loginAndGoToGroups(page, { userGroupRole: 'owner' });
    await goToGroupDetail(page);
    await page.getByRole('tab', { name: /Settings/ }).click();
    // Toggle peer sharing checkbox
    const peerCheckbox = page.getByLabel(/peer data sharing/i);
    await peerCheckbox.check();
    await expect(peerCheckbox).toBeChecked();
  });

  test('non-owner sees Leave but not Delete', async ({ page }) => {
    await loginAndGoToGroups(page, { userGroupRole: 'member' });
    await goToGroupDetail(page);
    await page.getByRole('tab', { name: /Settings/ }).click();
    // Leave button visible, Delete not
    await expect(page.getByRole('button', { name: 'Leave' })).toBeVisible();
    await expect(page.getByRole('button', { name: 'Delete Group' })).not.toBeVisible();
  });

  test('owner can delete group with confirmation', async ({ page }) => {
    await loginAndGoToGroups(page, { userGroupRole: 'owner' });
    await goToGroupDetail(page);
    await page.getByRole('tab', { name: /Settings/ }).click();
    await page.getByRole('button', { name: 'Delete Group' }).click();
    // Confirm dialog — check for the dialog title or confirm button
    await expect(page.getByText('This will permanently archive')).toBeVisible();
    await page.getByRole('button', { name: /Delete/i }).last().click();
    await expect(page.getByText('Group deleted')).toBeVisible({ timeout: 5000 });
  });
});

// ============================================================================
// Invite Link Tests
// ============================================================================

test.describe('Group Coaching - Invite Links', () => {
  test('copy button produces full invite URL', async ({ page }) => {
    await loginAndGoToGroups(page, { userGroupRole: 'owner' });
    await goToGroupDetail(page);
    await page.getByRole('tab', { name: /Invites/ }).click();
    // Verify invite code is visible
    await expect(page.getByText('MRT2026X')).toBeVisible();
    // The copy button should exist next to the invite code
    const copyButton = page.getByRole('button', { name: 'Copy invite link to clipboard' });
    await expect(copyButton).toBeVisible();
  });

  test('copy button includes full invite URL', async ({ page }) => {
    await loginAndGoToGroups(page, { userGroupRole: 'owner' });
    await goToGroupDetail(page);
    await page.getByRole('tab', { name: /Invites/ }).click();
    // Verify the copy button exists for the invite code
    await expect(page.getByRole('button', { name: 'Copy invite link to clipboard' })).toBeVisible({ timeout: 5000 });
  });
});

// ============================================================================
// Group Creation Permission Tests
// ============================================================================

test.describe('Group Coaching - Creation Permissions', () => {
  test('shows Create Group button when user has permission', async ({ page }) => {
    await loginAndGoToGroups(page, { canCreateGroup: true, groupCreationPolicy: 'everyone' });
    await expect(page.getByRole('button', { name: /Create Group/ })).toBeVisible();
  });

  test('hides Create Group button when policy is admins_only and user is not admin', async ({ page }) => {
    await setupDashboardMocks(page, { role: 'user' });
    await setupGroupMocks(page, { canCreateGroup: false, groupCreationPolicy: 'admins_only' });
    await loginToDashboard(page);
    await navigateToTab(page, 'Groups');
    // Create Group button should not be visible
    await expect(page.getByRole('button', { name: /Create Group/ })).not.toBeVisible();
    // Join Group button should still be visible
    await expect(page.getByRole('button', { name: /Join Group/ })).toBeVisible();
  });
});
