// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: E2E for the coach's group and invite steps — name and agent on one screen, then the link to share
// ABOUTME: A coach with and without coach access (the seat card tells the truth), the QR behind a link, and putting the step off

import { test, expect, type Page, type Route } from '@playwright/test';
import { setupDashboardMocks, signInThroughHostedPage } from './test-helpers';

const json = (route: Route, status: number, body: unknown) =>
  route.fulfill({ status, contentType: 'application/json', body: JSON.stringify(body) });

/**
 * A connected coach whose agent and seat steps are done, so the flow opens on
 * the group step. `managesRoster` is their coach access (a TrainingPeaks
 * coach account, say): it decides whether the created group names them its
 * coach, and so whether the invite leaves for the coach Home.
 */
async function setupCoach(page: Page, managesRoster: boolean) {
  const calls = {
    createGroup: [] as unknown[],
    invites: [] as unknown[],
    steps: [] as string[],
    accessRequests: [] as unknown[],
  };
  let accessRequest: Record<string, unknown> | null = null;

  await page.addInitScript(() => {
    window.localStorage.setItem('dravr.profile_type_chosen.user-123', '1');
    window.localStorage.setItem('dravr.coach_proposal_done.user-123', '1');
    window.localStorage.setItem('dravr.coach_seat_done.user-123', '1');
    window.localStorage.setItem('dravr.theme', 'dark');
  });
  await setupDashboardMocks(page, { role: 'user', email: 'coach@test.com', displayName: 'Coach', managesRoster });

  // Registered after the dashboard defaults, so these win.
  await page.route('**/api/me/onboarding-status', (route) =>
    json(route, 200, {
      needs_provider_connection: false,
      steps: [],
      chosen_channel: null,
      coaches_others: true,
    }),
  );
  await page.route('**/api/me/onboarding/steps/**', async (route) => {
    calls.steps.push(`${route.request().url().split('/').pop()}:${route.request().postDataJSON().status}`);
    await route.fulfill({ status: 204, body: '' });
  });
  // The catalogue the group's agent is picked from.
  await page.route('**/api/agents**', async (route) => {
    if (route.request().method() !== 'GET') return route.fallback();
    await json(route, 200, {
      agents: [
        { id: 'a-1', title: 'Endurance Agent', description: null, category: 'training', tags: [] },
        { id: 'a-2', title: 'Triathlon Agent', description: null, category: 'training', tags: [] },
      ],
      total: 2,
      metadata: { timestamp: new Date().toISOString(), api_version: 'v1' },
    });
  });
  await page.route('**/api/groups', async (route) => {
    calls.createGroup.push(route.request().postDataJSON());
    await json(route, 201, {
      id: 'g-1',
      tenant_id: 'user-123',
      name: 'Les Rouleurs',
      description: null,
      agent_id: 'a-2',
      owner_id: 'user-123',
      coach_user_id: managesRoster ? 'user-123' : null,
    });
  });
  const invite = {
    id: 'i-1',
    group_id: 'g-1',
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
  let groupMade = false;
  await page.route('**/api/groups/g-1/invites', async (route) => {
    // Coach Home reads the group's invites back; the step creates one.
    if (route.request().method() === 'GET') return json(route, 200, { invites: [invite] });
    calls.invites.push(route.request().postDataJSON());
    groupMade = true;
    await json(route, 201, invite);
  });
  // The coach's roster once the group exists: the group, nobody in it yet.
  await page.route('**/api/coach/roster**', (route) =>
    json(route, 200, {
      scope: { visible_members: 0, roster_members: 0 },
      groups: groupMade ? [{ id: 'g-1', name: 'Les Rouleurs', athlete_count: 0 }] : [],
      athletes: [],
      next_offset: null,
    }),
  );
  // The coach's coach-access request: none until they ask, pending after.
  await page.route('**/api/me/coach-access-request', async (route) => {
    if (route.request().method() === 'POST') {
      calls.accessRequests.push(route.request().postDataJSON());
      accessRequest = {
        id: 'r-1',
        user_id: 'user-123',
        group_id: 'g-1',
        group_tenant_id: 'user-123',
        status: 'pending',
        created_at: new Date().toISOString(),
        decided_at: null,
        decided_by: null,
      };
      return json(route, 201, { request: accessRequest });
    }
    await json(route, 200, { request: accessRequest });
  });
  // The group's thread, made in the same step with the chosen agent: on the
  // chat list once it exists, so the dashboard can open it.
  let threadMade = false;
  const groupThread = {
    id: 'c-1',
    title: 'Les Rouleurs',
    agent_id: 'a-2',
    agent_title: 'Triathlon Agent',
    group_id: 'g-1',
    group_name: 'Les Rouleurs',
    message_count: 0,
    unread_count: 0,
    created_at: new Date().toISOString(),
    updated_at: new Date().toISOString(),
    last_message: null,
  };
  await page.route(/\/api\/chat\/conversations(\?.*)?$/, async (route) => {
    if (route.request().method() === 'POST') {
      threadMade = true;
      return json(route, 201, groupThread);
    }
    const conversations = threadMade ? [groupThread] : [];
    await json(route, 200, { conversations, total: conversations.length, limit: 50, offset: 0 });
  });
  await page.route('**/api/chat/conversations/c-1/messages', (route) =>
    json(route, 200, { messages: [] }),
  );

  await page.goto('/');
  await signInThroughHostedPage(page);
  await expect(page.getByTestId('onboarding-group-name')).toBeVisible({ timeout: 15_000 });

  return calls;
}

/** Name the group, change its agent from the default, create it — one screen. */
async function createGroup(page: Page) {
  await page.getByTestId('onboarding-group-name').fill('Les Rouleurs');
  await expect(page.getByTestId('onboarding-group-agent')).toContainText('Endurance Agent');
  await page.getByRole('button', { name: 'Choose another agent for the group' }).click();
  await page.getByTestId('onboarding-group-agent-a-2').click();
  await expect(page.getByTestId('onboarding-group-agent')).toContainText('Triathlon Agent');
  await page.getByRole('button', { name: 'Create the group' }).click();
}

test('a coach with a TrainingPeaks seat leaves onboarding with a group, its link and its QR code, for the coach Home', async ({
  page,
}) => {
  const calls = await setupCoach(page, true);

  await createGroup(page);

  await expect(page.getByTestId('onboarding-group-link')).toContainText('/groups/join/ABCD2345');
  // One job: the link. The QR waits behind a link.
  await expect(page.getByTestId('onboarding-group-qr')).toHaveCount(0);
  await page.getByRole('button', { name: 'Show QR code' }).click();
  await expect(page.getByTestId('onboarding-group-qr')).toBeVisible();
  await expect(page.getByTestId('onboarding-group-seat')).toHaveAttribute('data-state', 'coach');
  await expect(page.getByTestId('onboarding-group-seat')).toContainText("You're this group's coach");
  expect(calls.createGroup).toEqual([
    { name: 'Les Rouleurs', agent_id: 'a-2', coach_is_me: true },
  ]);
  expect(calls.invites).toEqual([{ expires_in_days: 30 }]);

  // An active seat leaves for the coach Home, where the athlete invited waits.
  const done = page.getByTestId('onboarding-group-done');
  await expect(done).toHaveText('Open my coach Home');
  // Only an invitation says the seat was granted by the team.
  await expect(page.getByTestId('onboarding-group-seat-active')).toHaveCount(0);
  await done.click();
  await expect(page.getByTestId('onboarding-flow')).toHaveCount(0, { timeout: 10_000 });
  expect(calls.steps).toContain('coach_group:complete');
  await expect(page).toHaveURL(/#home$/);
  await expect(page.getByTestId('coach-home')).toBeVisible({ timeout: 10_000 });
  await expect(page.getByTestId('coach-pending-invite-g-1')).toContainText('Les Rouleurs');
});

test('a coach without coach access gets the group and asks for access in one tap', async ({
  page,
}) => {
  const calls = await setupCoach(page, false);

  await createGroup(page);

  // Nothing claims "pending" before a request exists.
  const seat = page.getByTestId('onboarding-group-seat');
  await expect(seat).toHaveAttribute('data-state', 'none');
  await expect(seat).toContainText("You're not this group's coach yet");
  await expect(seat).not.toContainText(/pending/i);
  await expect(page.getByRole('link', { name: 'Ask at support@dravr.ai' })).toHaveCount(0);

  // One tap asks for coach access, naming the group the coach just made.
  await page.getByRole('button', { name: 'Request coach access' }).click();
  await expect(seat).toHaveAttribute('data-state', 'requested');
  // Dated, and no wait time is promised: the answer comes here and by email.
  await expect(seat).toContainText("We'll let you know here and by email.");
  await expect(seat).not.toContainText('24');
  expect(calls.accessRequests).toEqual([{ group_id: 'g-1' }]);

  // Without the seat, "Go to my group" lands in the group's thread, not on Home.
  await page.getByTestId('onboarding-group-done').click();
  await expect(page.getByTestId('onboarding-flow')).toHaveCount(0, { timeout: 10_000 });
  expect(calls.steps).toContain('coach_group:complete');
  await expect(page).toHaveURL(/#chat\/c-1$/);
  await expect(page.getByTestId('conversation-header-title')).toHaveText(/Les Rouleurs/, {
    timeout: 10_000,
  });
});

test('a coach can put the group step off', async ({ page }) => {
  const calls = await setupCoach(page, false);

  await page.getByRole('button', { name: 'Later' }).click();
  await expect(page.getByTestId('onboarding-flow')).toHaveCount(0, { timeout: 10_000 });
  // The step is recorded fire-and-forget, so its request can land after the flow closes.
  await expect.poll(() => calls.steps).toContain('coach_group:skipped');
  expect(calls.createGroup).toEqual([]);
  // Later lands where it always has: the dashboard's default, Home.
  await expect(page).toHaveURL(/#home$/);
});
