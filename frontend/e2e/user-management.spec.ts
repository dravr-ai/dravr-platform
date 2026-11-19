// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Playwright E2E tests for user management features.
// ABOUTME: Tests user listing, approval, suspension, password reset, user details and search, on rows shaped as the server sends them.

import { test, expect, type Locator, type Page } from '@playwright/test';
import type { UserRole, UserStatus } from '@pierre/shared-types';
import { setupDashboardMocks, loginToDashboard, navigateToTab } from './test-helpers';

/**
 * A row of `GET /api/admin/users` and `GET /api/admin/pending-users`, field for
 * field the server's `UserSummary` (crates/pierre-routes-admin/src/handlers/users.rs):
 * the status is `user_status`, the last visit `last_active`, and the tier the
 * display form (`Starter`).
 */
interface UserRow {
  id: string;
  email: string;
  display_name: string | null;
  tier: 'Starter' | 'Professional' | 'Enterprise';
  created_at: string;
  last_active: string;
  user_status: UserStatus;
  role: UserRole;
  is_admin: boolean;
  approved_at: string | null;
  approved_by: string | null;
}

/** The account `setupDashboardMocks` signs in as. */
const SIGNED_IN_ADMIN_ID = 'user-123';

function userRow(
  fields: Pick<UserRow, 'id' | 'email' | 'display_name' | 'user_status'> &
    Partial<Pick<UserRow, 'tier' | 'role'>>,
): UserRow {
  const role = fields.role ?? 'user';
  const approved = fields.user_status === 'pending' ? null : '2024-01-02T10:00:00+00:00';
  return {
    id: fields.id,
    email: fields.email,
    display_name: fields.display_name,
    tier: fields.tier ?? 'Starter',
    created_at: '2024-01-01T10:00:00+00:00',
    last_active: '2024-01-20T15:30:00+00:00',
    user_status: fields.user_status,
    role,
    is_admin: role === 'admin',
    approved_at: approved,
    approved_by: approved ? SIGNED_IN_ADMIN_ID : null,
  };
}

const PENDING_1 = userRow({ id: 'user-1', email: 'pending1@example.com', display_name: 'Pending User 1', user_status: 'pending' });
const PENDING_2 = userRow({ id: 'user-2', email: 'pending2@example.com', display_name: 'Pending User 2', user_status: 'pending' });
const ACTIVE_USER = userRow({ id: 'user-3', email: 'active@example.com', display_name: 'Active User', user_status: 'active' });

// Helper to set up user management specific mocks
async function setupUserManagementMocks(
  page: Page,
  options: { pendingUsers?: UserRow[]; allUsers?: UserRow[] } = {},
) {
  const {
    pendingUsers = [PENDING_1, PENDING_2],
    allUsers = [PENDING_1, PENDING_2, ACTIVE_USER],
  } = options;

  // Set up base dashboard mocks (includes login mock)
  await setupDashboardMocks(page, { role: 'admin' });

  // Override pending users mock
  await page.route('**/api/admin/pending-users', async (route) => {
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      // The admin handler's `{ success, message, data }` envelope.
      body: JSON.stringify({
        success: true,
        message: `Retrieved ${pendingUsers.length} pending users`,
        data: { count: pendingUsers.length, users: pendingUsers },
      }),
    });
  });

  // The list itself, and only it: `/api/admin/users/{id}/...` is each
  // drawer call's own mock (`mockUserDrawer`), never this envelope.
  await page.route(
    (url) => url.pathname.endsWith('/api/admin/users'),
    async (route) => {
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({
          success: true,
          message: `Retrieved ${allUsers.length} users`,
          data: {
            users: allUsers,
            total: allUsers.length,
            has_more: false,
          },
        }),
      });
    },
  );
}

/**
 * What the user drawer reads about `user` when it opens, each in the shape its
 * endpoint answers: the rate limit and the activity in the admin envelope,
 * the usage, the admin profile and the feature overrides bare.
 */
async function mockUserDrawer(
  page: Page,
  user: UserRow,
  monthly: { limit: number; used: number } = { limit: 10000, used: 500 },
) {
  const fulfill = (pattern: string, body: unknown) =>
    page.route(pattern, async (route) => {
      await route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify(body) });
    });
  await fulfill(`**/api/admin/users/${user.id}/rate-limit`, {
    success: true,
    message: 'Rate limit information retrieved',
    data: {
      user_id: user.id,
      tier: user.tier,
      rate_limits: { monthly: { ...monthly, remaining: monthly.limit - monthly.used } },
      reset_times: { monthly_reset: '2024-02-01T00:00:00+00:00' },
      override_active: false,
      override_note: null,
    },
  });
  await fulfill(`**/api/admin/users/${user.id}/activity**`, {
    success: true,
    message: 'User activity retrieved',
    data: { user_id: user.id, period_days: 30, total_requests: monthly.used, top_tools: [] },
  });
  await fulfill(`**/api/admin/users/${user.id}/usage**`, {
    user_id: user.id,
    from: '2024-01-01T00:00:00+00:00',
    by_model: [],
    total_cost_usd: 0,
    daily: [],
  });
  await fulfill(`**/api/admin/users/${user.id}/admin-profile`, {
    user_id: user.id,
    coaching_persona: 'supportive_coach',
    manages_roster: false,
    manages_roster_operator_grant: null,
    installed_agents: [],
    joined_groups: [],
  });
  await fulfill(`**/api/admin/users/${user.id}/features`, { rows: [], known: [] });
}

async function loginAndNavigateToUsers(page: Page) {
  await loginToDashboard(page);
  await navigateToTab(page, 'Users');
  await page.waitForTimeout(500);
}

/** One of the Users screen's own tabs, whose name carries its count when it has one. */
function userTab(page: Page, name: 'Active' | 'Suspended' | 'All Users'): Locator {
  return page.getByRole('button', { name: new RegExp(`^${name}( \\d+)?$`) });
}

/** The account's row on the Active, Suspended and All Users tabs. */
function listRow(page: Page, user: UserRow): Locator {
  return page.locator('div.cursor-pointer').filter({ hasText: user.email });
}

/** Opens the account's drawer the way an admin does: by clicking its row. */
async function openDrawer(page: Page, user: UserRow): Promise<Locator> {
  await listRow(page, user).getByText(user.email).click();
  const drawer = page.getByRole('dialog', { name: 'User Details' });
  await expect(drawer).toBeVisible();
  return drawer;
}

test.describe('User Management - Pending Users', () => {
  test('displays pending users badge in sidebar', async ({ page }) => {
    await setupUserManagementMocks(page);
    await loginToDashboard(page);

    // Look for the Users button in the sidebar navigation (list item)
    // Use exact match to avoid matching the alert button
    const usersButton = page.getByRole('list').getByRole('button', { name: '2 Users', exact: true });
    await expect(usersButton).toBeVisible();

    // The button should contain the pending count
    await expect(usersButton).toContainText('2');
  });

  test('navigates to user management tab', async ({ page }) => {
    await setupUserManagementMocks(page);
    await loginAndNavigateToUsers(page);

    // Should see user management content - check for the h1 heading in Dashboard header bar
    await expect(page.getByRole('heading', { name: 'Users', level: 1 })).toBeVisible({ timeout: 5000 });
  });

  test('displays pending users list', async ({ page }) => {
    await setupUserManagementMocks(page);
    await loginAndNavigateToUsers(page);

    // Should see pending users
    await expect(page.getByText('pending1@example.com')).toBeVisible();
    await expect(page.getByText('pending2@example.com')).toBeVisible();
  });

  test('can switch between user status tabs', async ({ page }) => {
    await setupUserManagementMocks(page);
    await loginAndNavigateToUsers(page);

    // The screen opens on the Pending queue: the two sign-ups, and not the
    // active account.
    await expect(page.getByText(PENDING_1.email)).toBeVisible();
    await expect(page.getByText(PENDING_2.email)).toBeVisible();
    await expect(page.getByText(ACTIVE_USER.email)).toBeHidden();

    // Active lists the active account and neither sign-up.
    await userTab(page, 'Active').click();
    await expect(listRow(page, ACTIVE_USER)).toBeVisible();
    await expect(page.getByText(PENDING_1.email)).toBeHidden();
    await expect(page.getByText(PENDING_2.email)).toBeHidden();

    // All Users lists the three of them.
    await userTab(page, 'All Users').click();
    for (const user of [PENDING_1, PENDING_2, ACTIVE_USER]) {
      await expect(listRow(page, user)).toBeVisible();
    }
  });
});

test.describe('User Management - Approve User', () => {
  const PENDING = userRow({ id: 'user-1', email: 'pending@example.com', display_name: 'Pending User', user_status: 'pending' });

  test('shows approval confirmation modal', async ({ page }) => {
    await setupUserManagementMocks(page, { pendingUsers: [PENDING], allUsers: [PENDING] });

    await page.route('**/api/admin/approve-user/**', async (route) => {
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({
          success: true,
          message: 'User approved successfully',
          data: {
            user: { id: 'user-1', email: 'pending@example.com', user_status: 'active' },
            reason: 'Verified legitimate user',
          },
        }),
      });
    });

    await loginAndNavigateToUsers(page);

    // The Pending queue's own button: "Pre-approved" is a tab, not this.
    await page.getByRole('button', { name: 'Approve', exact: true }).click();

    const dialog = page.getByRole('dialog', { name: 'Approve User' });
    await expect(dialog).toBeVisible();
    await expect(dialog.getByText('pending@example.com')).toBeVisible();
    await dialog.getByLabel('Reason (optional)').fill('Verified legitimate user');

    const [request] = await Promise.all([
      page.waitForRequest(
        (sent) => sent.method() === 'POST' && sent.url().endsWith('/api/admin/approve-user/user-1'),
      ),
      dialog.getByRole('button', { name: 'Approve User' }).click(),
    ]);
    expect(request.postDataJSON()).toEqual({ reason: 'Verified legitimate user' });
    await expect(dialog).toBeHidden();
  });
});

test.describe('User Management - Suspend User', () => {
  const ACTIVE = userRow({ id: 'user-1', email: 'active@example.com', display_name: 'Active User', user_status: 'active' });

  async function mockSuspend(page: Page, user: UserRow) {
    await page.route(`**/api/admin/suspend-user/${user.id}`, async (route) => {
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({
          success: true,
          message: 'User suspended successfully',
          data: {
            user: { id: user.id, email: user.email, user_status: 'suspended' },
            reason: 'No reason provided',
          },
        }),
      });
    });
  }

  test('can suspend an active user', async ({ page }) => {
    await setupUserManagementMocks(page, { pendingUsers: [], allUsers: [ACTIVE] });
    await mockSuspend(page, ACTIVE);

    await loginAndNavigateToUsers(page);
    await userTab(page, 'Active').click();

    // The row's own button: "Suspended" is a tab, not this.
    await listRow(page, ACTIVE).getByRole('button', { name: 'Suspend', exact: true }).click();

    const dialog = page.getByRole('dialog', { name: 'Suspend User' });
    await expect(dialog).toBeVisible();
    await expect(dialog.getByText('active@example.com')).toBeVisible();

    const [request] = await Promise.all([
      page.waitForRequest(
        (sent) => sent.method() === 'POST' && sent.url().endsWith('/api/admin/suspend-user/user-1'),
      ),
      dialog.getByRole('button', { name: 'Suspend User' }).click(),
    ]);
    // No reason typed: the body carries none.
    expect(request.postDataJSON()).toEqual({});
    await expect(dialog).toBeHidden();
  });

  test("offers Suspend on a pending account and another admin, never on the signed-in admin's own", async ({ page }) => {
    const self = userRow({
      id: SIGNED_IN_ADMIN_ID,
      email: 'admin@test.com',
      display_name: 'Test Admin',
      user_status: 'active',
      role: 'admin',
      tier: 'Professional',
    });
    const otherAdmin = userRow({ id: 'admin-2', email: 'admin2@example.com', display_name: 'Other Admin', user_status: 'active', role: 'admin' });
    const pending = userRow({ id: 'user-1', email: 'pending@example.com', display_name: 'Pending User', user_status: 'pending' });
    const suspended = userRow({ id: 'user-2', email: 'suspended@example.com', display_name: 'Suspended User', user_status: 'suspended' });
    await setupUserManagementMocks(page, {
      pendingUsers: [pending],
      allUsers: [self, otherAdmin, pending, suspended],
    });
    await mockUserDrawer(page, self);
    await mockUserDrawer(page, pending);

    await loginAndNavigateToUsers(page);
    await userTab(page, 'All Users').click();

    const suspend = { name: 'Suspend', exact: true } as const;
    await expect(listRow(page, otherAdmin).getByRole('button', suspend)).toBeVisible();
    await expect(listRow(page, pending).getByRole('button', suspend)).toBeVisible();
    await expect(listRow(page, pending).getByRole('button', { name: 'Approve', exact: true })).toBeVisible();
    await expect(listRow(page, suspended).getByRole('button', { name: 'Reactivate' })).toBeVisible();
    await expect(listRow(page, suspended).getByRole('button', suspend)).toHaveCount(0);
    // The server refuses an admin its own suspension, so its row offers nothing.
    await expect(listRow(page, self)).toBeVisible();
    await expect(listRow(page, self).getByRole('button')).toHaveCount(0);

    // The drawer follows the same rule.
    const ownDrawer = await openDrawer(page, self);
    await expect(ownDrawer.getByRole('button', { name: 'Reset Password' })).toBeVisible();
    await expect(ownDrawer.getByRole('button', { name: 'Suspend User' })).toHaveCount(0);
    await ownDrawer.getByRole('button', { name: 'Close drawer' }).click();
    await expect(ownDrawer).toBeHidden();

    const pendingDrawer = await openDrawer(page, pending);
    await expect(pendingDrawer.getByRole('button', { name: 'Approve User' })).toBeVisible();
    await expect(pendingDrawer.getByRole('button', { name: 'Suspend User' })).toBeVisible();
  });
});

test.describe('User Management - User Details', () => {
  const DETAILED = userRow({
    id: 'user-1',
    email: 'user@example.com',
    display_name: 'Test User',
    user_status: 'active',
    tier: 'Professional',
  });

  test('can view user details drawer', async ({ page }) => {
    await setupUserManagementMocks(page, { pendingUsers: [], allUsers: [DETAILED] });
    await mockUserDrawer(page, DETAILED, { limit: 100000, used: 5000 });

    await loginAndNavigateToUsers(page);
    await userTab(page, 'All Users').click();

    // The drawer is the account's own: its name, email, status and tier as
    // the list row sent them.
    const drawer = await openDrawer(page, DETAILED);
    await expect(drawer.getByRole('heading', { name: 'Test User' })).toBeVisible();
    await expect(drawer.getByText('user@example.com')).toBeVisible();
    await expect(drawer.getByText('active', { exact: true })).toBeVisible();
    await expect(drawer.getByText('Professional', { exact: true })).toBeVisible();

    await drawer.getByRole('button', { name: 'Close drawer' }).click();
    await expect(drawer).toBeHidden();
  });

  test('displays user rate limits', async ({ page }) => {
    await setupUserManagementMocks(page, { pendingUsers: [], allUsers: [DETAILED] });
    await mockUserDrawer(page, DETAILED, { limit: 100000, used: 5000 });

    await loginAndNavigateToUsers(page);
    await userTab(page, 'Active').click();

    const drawer = await openDrawer(page, DETAILED);
    await expect(drawer.getByText('user@example.com')).toBeVisible();
    await expect(drawer.getByText('Monthly Usage')).toBeVisible();
    await expect(drawer.getByText('5,000 / 100,000')).toBeVisible();
  });
});

test.describe('User Management - Admin role', () => {
  const RISING = userRow({ id: 'user-7', email: 'rising@example.com', display_name: 'Rising User', user_status: 'active' });

  test('makes a user an admin from the drawer once the change is confirmed', async ({ page }) => {
    await setupUserManagementMocks(page, { pendingUsers: [], allUsers: [RISING] });
    await mockUserDrawer(page, RISING);
    let promoted = 0;
    await page.route(`**/api/admin/users/${RISING.id}/promote`, async (route) => {
      promoted += 1;
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({
          success: true,
          message: 'User promoted to admin successfully',
          user: { id: RISING.id, email: RISING.email, is_admin: true, role: 'admin' },
        }),
      });
    });

    await loginAndNavigateToUsers(page);
    await userTab(page, 'All Users').click();
    const drawer = await openDrawer(page, RISING);

    await drawer.getByTestId('admin-role-change').click();
    const confirm = drawer.getByTestId('admin-role-confirm');
    await expect(confirm).toContainText('rising@example.com gets every admin power');
    expect(promoted).toBe(0);

    await confirm.getByRole('button', { name: 'Make admin' }).click();

    await expect(drawer.getByTestId('admin-role-change')).toHaveText('Remove admin role');
    expect(promoted).toBe(1);
  });
});

test.describe('User Management - Password Reset', () => {
  const TARGET = userRow({ id: 'user-1', email: 'user@example.com', display_name: 'Test User', user_status: 'active' });

  async function mockReset(page: Page, resetToken: string) {
    await page.route(`**/api/admin/users/${TARGET.id}/reset-password`, async (route) => {
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({
          success: true,
          message: 'Password reset token issued',
          data: {
            user_id: TARGET.id,
            email: TARGET.email,
            reset_token: resetToken,
            expires_in_seconds: 3600,
            reset_by: SIGNED_IN_ADMIN_ID,
            note: 'Deliver this token to the user.',
          },
        }),
      });
    });
  }

  /** The reset dialog, opened from the account's drawer. */
  async function openResetDialog(page: Page): Promise<Locator> {
    await loginAndNavigateToUsers(page);
    await userTab(page, 'Active').click();
    const drawer = await openDrawer(page, TARGET);
    await drawer.getByRole('button', { name: 'Reset Password' }).click();
    const dialog = page.getByRole('dialog', { name: 'Reset User Password' });
    await expect(dialog).toBeVisible();
    return dialog;
  }

  test('can reset user password', async ({ page }) => {
    await setupUserManagementMocks(page, { pendingUsers: [], allUsers: [TARGET] });
    await mockUserDrawer(page, TARGET);
    await mockReset(page, 'sel123.verifier123');

    const dialog = await openResetDialog(page);
    await expect(dialog.getByText('user@example.com')).toBeVisible();

    const [request] = await Promise.all([
      page.waitForRequest(
        (sent) => sent.method() === 'POST' && sent.url().endsWith('/api/admin/users/user-1/reset-password'),
      ),
      dialog.getByRole('button', { name: 'Reset Password' }).click(),
    ]);
    expect(request.postData()).toBeNull();
    await expect(dialog.getByText('Password Reset Successful')).toBeVisible();
  });

  test('displays the issued reset token in modal', async ({ page }) => {
    await setupUserManagementMocks(page, { pendingUsers: [], allUsers: [TARGET] });
    await mockUserDrawer(page, TARGET);
    await mockReset(page, 'sel456.verifier456');

    const dialog = await openResetDialog(page);
    await dialog.getByRole('button', { name: 'Reset Password' }).click();

    // Modal shows the token the server issued, read from the data envelope
    await expect(dialog.locator('code.font-mono')).toHaveText('sel456.verifier456');
    await expect(dialog.getByText(/^Expires: /)).toBeVisible();
  });
});

test.describe('User Management - Search', () => {
  const JOHN = userRow({ id: 'user-1', email: 'john@example.com', display_name: 'John Doe', user_status: 'active' });
  const JANE = userRow({ id: 'user-2', email: 'jane@example.com', display_name: 'Jane Smith', user_status: 'active', tier: 'Professional' });
  const BOB = userRow({ id: 'user-3', email: 'bob@test.com', display_name: 'Bob Wilson', user_status: 'suspended' });

  /** The search box, which the Active, Suspended and All Users tabs carry. */
  async function openSearch(page: Page): Promise<Locator> {
    await setupUserManagementMocks(page, { pendingUsers: [], allUsers: [JOHN, JANE, BOB] });
    await loginAndNavigateToUsers(page);
    await userTab(page, 'All Users').click();
    await expect(listRow(page, BOB)).toBeVisible();
    return page.getByRole('searchbox', { name: 'Search users by email or name' });
  }

  test('can search users by email', async ({ page }) => {
    const searchInput = await openSearch(page);
    await searchInput.fill('john@');

    // Should filter results
    await expect(listRow(page, JOHN)).toBeVisible();
    await expect(listRow(page, JANE)).toHaveCount(0);
    await expect(listRow(page, BOB)).toHaveCount(0);
    await expect(page.getByText('1 users', { exact: true })).toBeVisible();
  });

  test('can search users by name', async ({ page }) => {
    const searchInput = await openSearch(page);
    await searchInput.fill('Jane');

    await expect(listRow(page, JANE).getByText('Jane Smith')).toBeVisible();
    await expect(listRow(page, JOHN)).toHaveCount(0);
    await expect(listRow(page, BOB)).toHaveCount(0);
  });

  test('shows no results message when search finds nothing', async ({ page }) => {
    const searchInput = await openSearch(page);
    await searchInput.fill('nonexistent@user.com');

    // Should show no results message
    await expect(page.getByText('No users found', { exact: true })).toBeVisible();
    await expect(page.getByText('Try adjusting your search terms')).toBeVisible();
    await expect(page.locator('div.cursor-pointer')).toHaveCount(0);
  });
});
