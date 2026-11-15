// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: E2E for the team-issued coach-seat invitation (carnet#1524) — the console issues it, the link creates the coach account
// ABOUTME: Then two screens (group, invite) with Profile and Coach seat settled by the invitation, ending on coach Home with the athlete invited

import { test, expect, type Page, type Route } from '@playwright/test';
import { navigateToTab, setupDashboardMocks, signInThroughHostedPage, waitForLoginScreen } from './test-helpers';

const json = (route: Route, status: number, body: unknown) =>
  route.fulfill({ status, contentType: 'application/json', body: JSON.stringify(body) });

const TOKEN = 'AbCd1234EfGh5678.IjKl9012MnOp3456QrSt7890UvWx1234';

test.describe('Coach-seat invitation link', () => {
  async function setupLink(page: Page) {
    await page.route('**/admin/setup/status', (route) =>
      json(route, 200, { needs_setup: false, admin_user_exists: true, message: 'Admin user configured' }),
    );
    await page.route('**/api/auth/session', (route) => json(route, 401, {}));
  }

  test('words the form for a coach, then says the seat is already active', async ({ page }) => {
    await setupLink(page);
    let sent: Record<string, unknown> | null = null;
    await page.route('**/api/auth/accept-invitation', async (route) => {
      sent = route.request().postDataJSON() as Record<string, unknown>;
      await json(route, 201, {
        user_id: 'coach-invited',
        email: 'philippe@example.com',
        user_status: 'active',
        display_name: 'Philippe',
        message: 'Your account is ready.',
      });
    });

    await page.goto(`/accept-invite#token=${TOKEN}&kind=coach_seat`);
    await expect(page.locator('h1')).toContainText('Create your coach account'); // humanCoach.seatInvite.title
    await expect(page.getByText('An invitation from the Dravr team: your coach seat is included.')).toBeVisible(); // humanCoach.seatInvite.subtitle
    // The address is the invitation's: the form never asks for one.
    await expect(page.locator('input[name="email"]')).toHaveCount(0);
    await expect(page).toHaveURL((url) => url.pathname === '/' && url.hash === '');

    await page.locator('input[name="displayName"]').fill('Philippe');
    await page.locator('input[name="password"]').fill('SecurePassword123');
    await page.locator('input[name="confirmPassword"]').fill('SecurePassword123');
    await page.getByRole('button', { name: 'Create Account' }).click();

    await expect(page.locator('h1')).toContainText('Your coach account is ready');
    await expect(page.getByTestId('accepted-email')).toContainText('philippe@example.com');
    expect(sent).toMatchObject({ token: TOKEN, password: 'SecurePassword123' });
    await page.getByRole('button', { name: 'Continue to sign in' }).click();
    await waitForLoginScreen(page, 10000);
  });

  test('refuses an expired or revoked link plainly', async ({ page }) => {
    await setupLink(page);
    await page.route('**/api/auth/accept-invitation', (route) =>
      json(route, 404, { code: 'resource_not_found', message: 'The requested resource was not found' }),
    );

    await page.goto(`/accept-invite#token=${TOKEN}&kind=coach_seat`);
    await page.locator('input[name="password"]').fill('SecurePassword123');
    await page.locator('input[name="confirmPassword"]').fill('SecurePassword123');
    await page.getByRole('button', { name: 'Create Account' }).click();

    await expect(page.locator('h1')).toContainText('This invitation link no longer works');
    await expect(page.getByTestId('invite-expired-body')).toContainText('Ask the Dravr team to send you a new one');
  });
});

/**
 * A coach whose account a coach-seat invitation just created: the server
 * recorded the role and the seat, so the flow opens on the group step.
 */
async function setupInvitedCoach(page: Page) {
  const calls = { createGroup: [] as unknown[], steps: [] as string[] };
  let groupMade = false;
  await page.addInitScript(() => window.localStorage.setItem('dravr.theme', 'dark'));
  await setupDashboardMocks(page, { role: 'user', email: 'philippe@example.com', displayName: 'Philippe', managesRoster: true });

  let groupDone = false;
  await page.route('**/api/me/onboarding-status', (route) =>
    json(route, 200, {
      needs_provider_connection: true,
      chosen_channel: null,
      coaches_others: true,
      settled_by_invite: true,
      steps: [
        { step_id: 'about_you', status: 'not_applicable' },
        { step_id: 'parq', status: 'not_applicable' },
        { step_id: 'profile_type', status: 'complete' },
        { step_id: 'coach_seat', status: 'complete' },
        ...(groupDone ? [{ step_id: 'coach_group', status: 'complete' }] : []),
      ],
    }),
  );
  await page.route('**/api/me/onboarding/steps/**', async (route) => {
    const step = route.request().url().split('/').pop() ?? '';
    calls.steps.push(`${step}:${route.request().postDataJSON().status}`);
    if (step === 'coach_group') groupDone = true;
    await route.fulfill({ status: 204, body: '' });
  });
  await page.route('**/api/agents**', async (route) => {
    if (route.request().method() !== 'GET') return route.fallback();
    await json(route, 200, {
      agents: [{ id: 'a-1', title: 'Endurance Agent', description: null, category: 'training', tags: ['endurance'] }],
      total: 1,
      metadata: { timestamp: new Date().toISOString(), api_version: 'v1' },
    });
  });
  const group = {
    id: 'g-lea',
    tenant_id: 'user-123',
    name: 'Léa · Marathon de Montréal',
    description: 'Marathon de Montréal',
    agent_id: 'a-1',
    owner_id: 'user-123',
    coach_user_id: 'user-123',
  };
  await page.route('**/api/groups', async (route) => {
    calls.createGroup.push(route.request().postDataJSON());
    groupMade = true;
    await json(route, 201, group);
  });
  const invite = {
    id: 'i-1',
    group_id: 'g-lea',
    tenant_id: 'user-123',
    code: 'ABCD2345',
    kind: 'member',
    created_by: 'user-123',
    expires_at: new Date(Date.now() + 30 * 86_400_000).toISOString(),
    max_uses: null,
    use_count: 0,
    is_active: true,
    approves_signup: false,
    created_at: new Date().toISOString(),
  };
  await page.route('**/api/groups/g-lea/invites', async (route) => {
    if (route.request().method() === 'POST') return json(route, 201, invite);
    await json(route, 200, { invites: [invite] });
  });
  let threadMade = false;
  const thread = {
    id: 'c-lea',
    title: group.name,
    agent_id: 'a-1',
    agent_title: 'Endurance Agent',
    group_id: 'g-lea',
    group_name: group.name,
    message_count: 0,
    unread_count: 0,
    created_at: new Date().toISOString(),
    updated_at: new Date().toISOString(),
    last_message: null,
  };
  await page.route(/\/api\/chat\/conversations(\?.*)?$/, async (route) => {
    if (route.request().method() === 'POST') {
      threadMade = true;
      return json(route, 201, thread);
    }
    const conversations = threadMade ? [thread] : [];
    await json(route, 200, { conversations, total: conversations.length, limit: 50, offset: 0 });
  });
  // The coach's roster: the group, nobody in it yet.
  await page.route('**/api/coach/roster**', (route) =>
    json(route, 200, {
      scope: { visible_members: 0, roster_members: 0 },
      groups: groupMade ? [{ id: 'g-lea', name: group.name, athlete_count: 0 }] : [],
      athletes: [],
      next_offset: null,
    }),
  );

  await page.goto('/');
  await signInThroughHostedPage(page);
  await expect(page.getByTestId('onboarding-group-athlete')).toBeVisible({ timeout: 15_000 });
  return calls;
}

test('an invited coach meets two screens, the role and the seat settled, and lands on coach Home with the athlete invited', async ({
  page,
}) => {
  const calls = await setupInvitedCoach(page);

  // Screen 1 — the group. Profile and Coach seat stay on the bar, done, settled by the invitation.
  const bar = page.getByTestId('onboarding-progress');
  await expect(bar).toHaveAttribute('aria-label', 'Step 3 of 4: Your group');
  await expect(page.getByTestId('onboarding-settled-by-invite')).toHaveText(
    'Profile and Coach seat settled by your invitation',
  );
  await expect(page.locator('li[data-settled="invite"]')).toHaveCount(2);
  await expect(page.getByTestId('onboarding-group-seat-active')).toHaveText(
    'Your coach seat is active: granted by the Dravr team.',
  );
  // No role question, no seat request on the way.
  await expect(page.getByTestId('coach-access-request')).toHaveCount(0);

  await page.getByTestId('onboarding-group-athlete').fill('Léa');
  await page.getByTestId('onboarding-group-goal').fill('Marathon de Montréal');
  await expect(page.getByTestId('onboarding-group-agent')).toContainText('Endurance Agent');
  await page.getByRole('button', { name: 'Create the group' }).click();

  // Screen 2 — the invite.
  await expect(bar).toHaveAttribute('aria-label', 'Step 4 of 4: Invite');
  await expect(page.getByTestId('onboarding-group-link')).toContainText('/groups/join/ABCD2345');
  await expect(page.getByTestId('onboarding-group-seat')).toHaveAttribute('data-state', 'coach');
  await page.getByTestId('onboarding-group-done').click();

  // Coach Home, with the athlete invited and not joined yet.
  const row = page.getByTestId('coach-pending-invite-g-lea');
  await expect(row).toBeVisible({ timeout: 15_000 });
  await expect(page.getByTestId('coach-home')).toBeVisible();
  await expect(row.getByTestId('coach-pending-invite-status')).toHaveText('Invited · hasn’t joined yet');
  await expect(row.getByTestId('coach-pending-invite-copy')).toHaveText('Copy the link again');
  expect(calls.createGroup).toHaveLength(1);
  expect(calls.steps).toEqual(['coach_group:complete']);
});

test.describe('Coach-seat invitations in the console', () => {
  async function setupConsole(page: Page) {
    const posted: unknown[] = [];
    const revoked: string[] = [];
    let invitations = [
      {
        email: 'philippe@example.com',
        invited_by: 'admin-1',
        invited_by_email: 'team@dravr.ai',
        created_at: '2026-11-13T14:40:00Z',
        expires_at: '2026-11-20T14:40:00Z',
        expired: false,
      },
    ];
    await setupDashboardMocks(page, { role: 'super_admin' });
    await page.route('**/api/admin/coach-access-requests**', (route) =>
      json(route, 200, { success: true, message: '0 coach access request(s)', data: { requests: [], total: 0 } }),
    );
    await page.route('**/api/admin/coach-seat-invitations', async (route) => {
      if (route.request().method() === 'POST') {
        const body = route.request().postDataJSON() as { email: string };
        posted.push(body);
        if (body.email === 'member@example.com') {
          return json(route, 200, {
            success: true,
            message: 'This address already has an account: grant it coach access directly instead',
            data: { outcome: 'account_exists', email: body.email, user_id: 'u-9', display_name: null, manages_roster: false },
          });
        }
        invitations = [
          ...invitations,
          {
            email: body.email,
            invited_by: 'admin-1',
            invited_by_email: 'team@dravr.ai',
            created_at: '2026-11-13T15:00:00Z',
            expires_at: '2026-11-20T15:00:00Z',
            expired: false,
          },
        ];
        return json(route, 200, {
          success: true,
          message: `Coach-seat invitation sent to ${body.email}: its link works once, until 2026-11-20 15:00 UTC`,
          data: { outcome: 'invited', email: body.email, expires_at: '2026-11-20T15:00:00Z' },
        });
      }
      await json(route, 200, {
        success: true,
        message: `${invitations.length} coach-seat invitation(s)`,
        data: { invitations, total: invitations.length },
      });
    });
    await page.route('**/api/admin/coach-seat-invitations/*', async (route) => {
      const email = decodeURIComponent(route.request().url().split('/').pop() ?? '');
      revoked.push(email);
      invitations = invitations.filter((i) => i.email !== email);
      await json(route, 200, {
        success: true,
        message: `Coach-seat invitation for ${email} revoked: its link no longer works`,
        data: { email, revoked: true },
      });
    });
    await page.goto('/');
    await signInThroughHostedPage(page);
    await page.waitForSelector('main', { timeout: 15_000 });
    await navigateToTab(page, 'Users');
    await page.getByRole('button', { name: /Coach access/ }).first().click();
    return { posted, revoked };
  }

  test('lists the invited coach with its expiry, sends one, and revokes one', async ({ page }) => {
    const calls = await setupConsole(page);
    const row = page.getByTestId('coach-seat-invitation-philippe@example.com');
    await expect(row).toBeVisible();
    await expect(row).toContainText('invited');
    await expect(row).toContainText('Expires Nov 20, 2026');

    await page.getByTestId('coach-seat-invite-email').fill('new-coach@example.com');
    await page.getByTestId('coach-seat-invite-send').click();
    await expect(page.getByTestId('coach-seat-invite-result')).toContainText(
      'Coach-seat invitation sent to new-coach@example.com',
    );
    await expect(page.getByTestId('coach-seat-invitation-new-coach@example.com')).toBeVisible();

    await row.getByRole('button', { name: 'Revoke' }).click();
    await expect(row).toHaveCount(0);
    expect(calls.revoked).toEqual(['philippe@example.com']);
  });

  test('offers the direct grant for an address that already has an account', async ({ page }) => {
    await setupConsole(page);
    let granted: unknown = null;
    await page.route('**/api/admin/users/u-9/manages-roster', async (route) => {
      granted = route.request().postDataJSON();
      await json(route, 200, { success: true, message: 'ok', data: { user_id: 'u-9', manages_roster: true } });
    });

    await page.getByTestId('coach-seat-invite-email').fill('member@example.com');
    await page.getByTestId('coach-seat-invite-send').click();
    await expect(page.getByTestId('coach-seat-invite-result')).toContainText('already has an account');
    await page.getByTestId('coach-seat-invite-grant').click();
    await expect(page.getByTestId('coach-seat-invite-result')).toHaveText('member@example.com now has coach access');
    expect(granted).toEqual({ manages_roster: true });
  });
});
