// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: E2E for the coach's whole path — Profil, Place de coach, Ton groupe, Invitation — on a bar fixed at four
// ABOUTME: The seat says only what the server did: a dated request from « I use neither », coach access read back after TrainingPeaks

import { test, expect, type Page, type Route } from '@playwright/test';
import { setupDashboardMocks, signInThroughHostedPage } from './test-helpers';

const json = (route: Route, status: number, body: unknown) =>
  route.fulfill({ status, contentType: 'application/json', body: JSON.stringify(body) });

const card = (provider: string, display_name: string, extra: Record<string, unknown> = {}) => ({
  provider,
  display_name,
  description: `${display_name} activity data`,
  requires_oauth: false,
  connected: false,
  needs_reauth: false,
  capabilities: ['activities'],
  ...extra,
});

/**
 * A new account that has not answered the role question. The status answers
 * as the server does once the role is recorded; `trainingPeaksCoach` serves,
 * from then on, a TrainingPeaks coach account connected on the seat step, with
 * the session reporting the coach access the server granted for it.
 */
async function setupNewCoach(page: Page, { trainingPeaksCoach = false } = {}) {
  const calls = { steps: [] as string[], accessRequests: [] as unknown[], createGroup: [] as unknown[] };
  let rolePicked = false;
  let accessRequest: Record<string, unknown> | null = null;

  await page.addInitScript(() => {
    window.localStorage.setItem('dravr.theme', 'light');
  });
  await setupDashboardMocks(page, { role: 'user', email: 'coach@test.com', displayName: 'Philippe' });

  await page.route('**/api/me/onboarding-status', (route) =>
    json(route, 200, {
      // The TrainingPeaks account lands on the seat step, after the role.
      needs_provider_connection: !(trainingPeaksCoach && rolePicked),
      steps: rolePicked
        ? [
            { step_id: 'profile_type', status: 'complete' },
            { step_id: 'about_you', status: 'not_applicable' },
            { step_id: 'parq', status: 'not_applicable' },
          ]
        : [],
      chosen_channel: null,
      coaches_others: rolePicked,
    }),
  );
  await page.route('**/api/user/coaching-role', async (route) => {
    rolePicked = true;
    await json(route, 200, { coaches_others: true });
  });
  await page.route('**/api/me/onboarding/steps/**', async (route) => {
    calls.steps.push(`${route.request().url().split('/').pop()}:${route.request().postDataJSON().status}`);
    await route.fulfill({ status: 204, body: '' });
  });
  await page.route('**/api/providers', (route) =>
    json(route, 200, {
      providers: [
        card('sciotte', 'Strava'),
        card('intervals_icu', 'Intervals.icu'),
        card(
          'sciotte_trainingpeaks',
          'TrainingPeaks',
          trainingPeaksCoach && rolePicked ? { connected: true, account_role: 'coach' } : {},
        ),
      ],
    }),
  );
  await page.route('**/api/auth/session', (route) =>
    json(route, 200, {
      user: {
        id: 'user-123',
        user_id: 'user-123',
        email: 'coach@test.com',
        display_name: 'Philippe',
        role: 'user',
        is_admin: false,
        user_status: 'active',
        tenant_id: 'user-123',
        manages_roster: trainingPeaksCoach && rolePicked,
        analytics_consent: false,
        analytics_consent_answered: true,
      },
      access_token: 'test-jwt-token',
      csrf_token: 'test-csrf-token',
    }),
  );
  await page.route('**/api/me/coach-access-request', async (route) => {
    if (route.request().method() === 'POST') {
      calls.accessRequests.push(route.request().postDataJSON());
      accessRequest = {
        id: 'r-1',
        user_id: 'user-123',
        group_id: null,
        group_tenant_id: null,
        status: 'pending',
        created_at: '2026-11-12T19:32:00Z',
        decided_at: null,
        decided_by: null,
      };
      return json(route, 201, { request: accessRequest });
    }
    await json(route, 200, { request: accessRequest });
  });
  await page.route('**/api/agents**', async (route) => {
    if (route.request().method() !== 'GET') return route.fallback();
    await json(route, 200, {
      agents: [
        { id: 'a-1', title: 'Road Running Agent', description: null, category: 'training', tags: ['running'] },
        { id: 'a-2', title: 'Endurance Agent', description: 'Load, recovery and key sessions.', category: 'training', tags: ['endurance'] },
      ],
      total: 2,
      metadata: { timestamp: new Date().toISOString(), api_version: 'v1' },
    });
  });
  await page.route('**/api/groups', async (route) => {
    if (route.request().method() !== 'POST') return route.fallback();
    calls.createGroup.push(route.request().postDataJSON());
    await json(route, 201, {
      id: 'g-1',
      tenant_id: 'user-123',
      name: 'Léa · Marathon de Montréal',
      description: 'Marathon de Montréal',
      agent_id: 'a-2',
      owner_id: 'user-123',
      coach_user_id: trainingPeaksCoach ? 'user-123' : null,
    });
  });
  await page.route('**/api/groups/g-1/invites', (route) =>
    json(route, 201, { id: 'i-1', group_id: 'g-1', code: 'ABCD2345', kind: 'member' }),
  );
  let threadMade = false;
  const groupThread = {
    id: 'c-1',
    title: 'Léa · Marathon de Montréal',
    agent_id: 'a-2',
    agent_title: 'Endurance Agent',
    group_id: 'g-1',
    group_name: 'Léa · Marathon de Montréal',
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
  await page.route('**/api/chat/conversations/c-1/messages', (route) => json(route, 200, { messages: [] }));

  await page.goto('/');
  await signInThroughHostedPage(page);
  return calls;
}

/** The bar: four segments, its landmark naming the step the coach is on ("Step 2 of 4: Coach seat"). */
async function expectBar(page: Page, current: number, label: string) {
  const bar = page.getByRole('navigation', { name: `Step ${current} of 4: ${label}` });
  await expect(bar).toBeVisible();
  await expect(bar.locator('ol > li')).toHaveCount(4);
  return bar;
}

/**
 * Profil: a tap selects, Continue registers the answer. The bar on this first
 * screen still counts the athlete journey (the role is not known yet); it is
 * four from here on once that journey is four too.
 */
async function chooseCoach(page: Page) {
  await expect(page.getByRole('radio', { name: /I coach others/ })).toBeVisible({ timeout: 15_000 });
  await page.getByRole('radio', { name: /I coach others/ }).click();
  await page.getByRole('button', { name: 'Continue' }).click();
  await expect(page.getByRole('heading', { name: 'Get your coach seat' })).toBeVisible(); // humanCoach.seatHeading
}

test('a coach reaches the group thread in four screens, on a bar fixed at four', async ({ page }) => {
  const calls = await setupNewCoach(page);

  await chooseCoach(page);
  await expectBar(page, 2, 'Coach seat');
  // The two platforms as plain rows; nothing claimed yet.
  await expect(page.getByTestId('provider-card-sciotte_trainingpeaks')).toBeVisible();
  await expect(page.getByTestId('provider-card-intervals_icu')).toBeVisible();
  await expect(page.getByTestId('provider-card-sciotte')).toHaveCount(0);
  await expect(page.getByTestId('coach-seat-status')).toHaveCount(0);

  // « I use neither » files the request with no group, then dates it.
  await page.getByRole('button', { name: 'I use neither →' }).click();
  const status = page.getByTestId('coach-seat-status');
  await expect(status).toHaveAttribute('data-state', 'requested');
  await expect(status).toContainText('Request sent · usually within 24 h');
  expect(calls.accessRequests).toEqual([{}]);
  await page.getByRole('button', { name: 'Continue' }).click();

  // Ton groupe: one screen, name filled in from the athlete, agent defaulted.
  await expectBar(page, 3, 'Your group');
  await page.getByTestId('onboarding-group-athlete').fill('Léa');
  await page.getByTestId('onboarding-group-goal').fill('Marathon de Montréal');
  await expect(page.getByTestId('onboarding-group-name')).toHaveValue('Léa · Marathon de Montréal');
  await expect(page.getByTestId('onboarding-group-agent')).toContainText('Endurance Agent');
  await page.getByRole('button', { name: 'Create the group' }).click();

  // Invitation: one job, and the seat card dates the request.
  await expect(page.getByRole('heading', { name: 'Invite Léa' })).toBeVisible();
  await expectBar(page, 4, 'Invite');
  await expect(page.getByTestId('onboarding-group-seat')).toHaveAttribute('data-state', 'requested');
  expect(calls.createGroup).toEqual([
    { name: 'Léa · Marathon de Montréal', agent_id: 'a-2', coach_is_me: true, description: 'Marathon de Montréal' },
  ]);

  await page.getByRole('button', { name: "Go to Léa's group" }).click();
  await expect(page.getByTestId('onboarding-flow')).toHaveCount(0, { timeout: 10_000 });
  expect(calls.steps).toContain('coach_group:complete');
  await expect(page).toHaveURL(/#chat\/c-1$/);
});

test('a TrainingPeaks coach account grants the seat with no human in the loop, read back from the server', async ({
  page,
}) => {
  const calls = await setupNewCoach(page, { trainingPeaksCoach: true });

  await chooseCoach(page);
  const status = page.getByTestId('coach-seat-status');
  await expect(status).toHaveAttribute('data-state', 'coach', { timeout: 10_000 });
  await expect(status).toContainText("You're the coach of your groups");
  expect(calls.accessRequests).toEqual([]);
  await page.getByRole('button', { name: 'Continue' }).click();

  await page.getByTestId('onboarding-group-athlete').fill('Léa');
  await page.getByRole('button', { name: 'Create the group' }).click();
  await expect(page.getByTestId('onboarding-group-seat')).toHaveAttribute('data-state', 'coach');
  await expect(page.getByTestId('coach-access-request')).toHaveCount(0);
});

test('skipping the seat still shows it on the bar, and nothing is asked for', async ({ page }) => {
  const calls = await setupNewCoach(page);

  await chooseCoach(page);
  await page.getByRole('button', { name: 'Skip for now' }).click();
  const onGroup = await expectBar(page, 3, 'Your group');
  // A seat put off sets the same device flag as one settled and writes no
  // step row, so it keeps its segment and reads as behind the coach.
  await expect(onGroup.locator('ol > li').nth(1)).toHaveText('Coach seat');
  await expect(onGroup.locator('ol > li').nth(1)).toHaveAttribute('data-status', 'done');
  await expect(page.getByRole('heading', { name: 'Set up your group' })).toBeVisible();
  expect(calls.accessRequests).toEqual([]);
});
