// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: E2E for carnet#1530 — an athlete reaches Home in four screens: Profil, Tes données, Santé, Ton équipe
// ABOUTME: Pins the saved Connect skip, the one-tap health check, the invite-inferred role, and « Build my plan » asking the PAR-Q first

import { test, expect, type Page, type Route } from '@playwright/test';
import { ONBOARDED_STATUS, setupDashboardMocks, signInThroughHostedPage } from './test-helpers';

const json = (route: Route, body: unknown, status = 200) =>
  route.fulfill({ status, contentType: 'application/json', body: JSON.stringify(body) });

const PARQ = [
  'heart_condition',
  'chest_pain',
  'dizziness',
  'chronic_condition',
  'medication',
  'joint_problem',
  'supervised_only',
].map((id, i) => ({ id, text: `PAR-Q question ${i + 1}, word for word` }));

type Step = { step_id: string; status: string };

/**
 * A new athlete's server: the onboarding status is built from the step rows
 * the surfaces write, so a skip or an answer written on one screen is what
 * the next status read — and the next sign-in — sees.
 */
async function installServer(page: Page, start: { steps?: Step[]; coachesOthers?: boolean } = {}) {
  const state = {
    steps: [...(start.steps ?? [])] as Step[],
    coachesOthers: start.coachesOthers ?? false,
    roleWrites: [] as boolean[],
    parqAnswers: [] as Array<Array<{ id: string; yes: boolean }>>,
  };
  await page.route('**/api/me/onboarding-status', (route) =>
    json(route, {
      needs_provider_connection: true,
      steps: state.steps,
      chosen_channel: null,
      coaches_others: state.coachesOthers,
    }),
  );
  await page.route('**/api/me/onboarding/steps/**', (route) => {
    const stepId = new URL(route.request().url()).pathname.split('/').pop() ?? '';
    const { status } = route.request().postDataJSON() as { status: string };
    state.steps = [...state.steps.filter((s) => s.step_id !== stepId), { step_id: stepId, status }];
    return route.fulfill({ status: 204, body: '' });
  });
  await page.route('**/api/user/coaching-role', (route) => {
    const { coaches_others } = route.request().postDataJSON() as { coaches_others: boolean };
    state.roleWrites.push(coaches_others);
    state.coachesOthers = coaches_others;
    return json(route, { coaches_others });
  });
  await page.route('**/api/me/parq', (route) => {
    if (route.request().method() === 'GET') return json(route, { questions: PARQ });
    const { answers } = route.request().postDataJSON() as { answers: Array<{ id: string; yes: boolean }> };
    state.parqAnswers.push(answers);
    return json(route, { flags_raised: answers.filter((a) => a.yes).length });
  });
  const lead = { agent: { id: 'agent-mobility', title: 'Mobility', category: 'mobility' }, match_score: 0.6, reason: 'No data needed.' };
  await page.route('**/api/agents/proposal', (route) =>
    json(route, {
      profile: { has_profile: false, window_days: 28, total_activities: 0, primary_sport: null, sport_mix: [] },
      agents: [lead],
      team: { lead, members: [] },
    }),
  );
  return state;
}

async function signIn(page: Page, path = '/') {
  await page.goto(path);
  await signInThroughHostedPage(page);
}

const bar = (page: Page, name: string | RegExp) => page.getByRole('navigation', { name });

test.describe('athlete onboarding — four screens', () => {
  test('Profil, Tes données, Santé, Ton équipe: the bar reads 4 on every screen', async ({ page }) => {
    await setupDashboardMocks(page, { role: 'user', email: 'lea@test.com', displayName: 'Léa' });
    const server = await installServer(page);
    await signIn(page);

    // 1 · Profil: a tap selects, Continue registers.
    await expect(bar(page, 'Step 1 of 4: Profile')).toBeVisible({ timeout: 15_000 });
    await page.getByRole('radio', { name: /I'm an athlete/ }).click();
    expect(server.roleWrites).toEqual([]);
    await page.getByTestId('profile-type-continue').click();

    // 2 · Tes données: three services lead, the rest fold.
    await expect(bar(page, 'Step 2 of 4: Your data')).toBeVisible();
    await expect(page.getByRole('heading', { name: 'Connect your training' })).toBeVisible();
    expect(server.roleWrites).toEqual([false]);
    await page.getByRole('button', { name: 'Continue without connecting' }).click();

    // 3 · Santé: the seven questions, the common answer in one tap.
    await expect(bar(page, 'Step 3 of 4: Health')).toBeVisible();
    for (const q of PARQ) await expect(page.getByText(q.text)).toBeVisible();
    await page.getByRole('button', { name: 'None of these apply' }).click();

    // 4 · Ton équipe.
    await expect(bar(page, 'Step 4 of 4: Your team')).toBeVisible();
    await expect(page.getByRole('heading', { name: "Léa, here's your starting team" })).toBeVisible();
    expect(server.parqAnswers).toEqual([PARQ.map((q) => ({ id: q.id, yes: false }))]);
    expect(server.steps).toEqual(
      expect.arrayContaining([
        { step_id: 'connect_provider', status: 'skipped' },
        { step_id: 'parq', status: 'complete' },
      ]),
    );
    // Not one screen asked about the goal, the why or the check-in cadence.
    expect(server.steps.map((s) => s.step_id)).not.toContain('about_you');
    expect(server.steps.map((s) => s.step_id)).not.toContain('check_in');
  });

  test('« Continue without connecting » is saved: the next sign-in never shows Connect again', async ({ page }) => {
    await setupDashboardMocks(page, { role: 'user', email: 'lea@test.com', displayName: 'Léa' });
    await installServer(page, {
      steps: [
        { step_id: 'profile_type', status: 'complete' },
        { step_id: 'connect_provider', status: 'skipped' },
        { step_id: 'parq', status: 'complete' },
        { step_id: 'coach_proposal', status: 'complete' },
      ],
    });
    await signIn(page);

    await expect(page.locator('main')).toBeVisible({ timeout: 15_000 });
    await expect(page.getByRole('heading', { name: 'Connect your training' })).toHaveCount(0);
    await expect(page.getByTestId('onboarding-flow')).toHaveCount(0);
  });

  test('at 390px the health check answers in one tap, and every No/Yes target is 44px when some apply', async ({ page }) => {
    await page.setViewportSize({ width: 390, height: 844 });
    await setupDashboardMocks(page, { role: 'user', email: 'lea@test.com', displayName: 'Léa' });
    await installServer(page, {
      steps: [
        { step_id: 'profile_type', status: 'complete' },
        { step_id: 'connect_provider', status: 'skipped' },
      ],
    });
    await signIn(page);

    await expect(page.getByTestId('parq-none-apply')).toBeVisible({ timeout: 15_000 });
    expect((await page.getByTestId('parq-none-apply').boundingBox())?.height).toBeGreaterThanOrEqual(44);
    await page.getByTestId('parq-some-apply').click();
    for (const q of PARQ) {
      for (const answer of ['no', 'yes']) {
        const box = await page.getByTestId(`parq-${q.id}-${answer}`).boundingBox();
        expect(box?.height, `${q.id} ${answer}`).toBeGreaterThanOrEqual(44);
        expect(box?.width, `${q.id} ${answer}`).toBeGreaterThanOrEqual(44);
      }
    }
    const { scrollWidth, clientWidth } = await page.evaluate(() => ({
      scrollWidth: document.documentElement.scrollWidth,
      clientWidth: document.documentElement.clientWidth,
    }));
    expect(scrollWidth).toBeLessThanOrEqual(clientWidth + 1);
  });

  test('an athlete who arrived by an invite link starts on Tes données, Profil already done', async ({ page }) => {
    await setupDashboardMocks(page, { role: 'user', email: 'lea@test.com', displayName: 'Léa' });
    const server = await installServer(page);
    // The link opened signed out: the sign-in redirect must not lose it.
    await signIn(page, '/groups/join/ABCD2345');

    const onData = bar(page, 'Step 2 of 4: Your data');
    await expect(onData).toBeVisible({ timeout: 15_000 });
    await expect(onData.locator('li').first()).toHaveAttribute('data-status', 'done');
    expect(server.roleWrites).toEqual([false]);
    expect(server.steps).toContainEqual({ step_id: 'profile_type', status: 'complete' });
  });
});

test.describe('athlete Home — the questions the wizard left behind', () => {
  async function onboardedWith(page: Page, steps: Step[]) {
    await setupDashboardMocks(page, { role: 'user', email: 'lea@test.com', displayName: 'Léa' });
    const writes: Step[] = [];
    const parq: unknown[] = [];
    await page.route('**/api/me/onboarding-status', (route) =>
      json(route, { ...ONBOARDED_STATUS, steps: [...steps, ...writes] }),
    );
    await page.route('**/api/me/onboarding/steps/**', (route) => {
      const stepId = new URL(route.request().url()).pathname.split('/').pop() ?? '';
      writes.push({ step_id: stepId, ...(route.request().postDataJSON() as { status: string }) });
      return route.fulfill({ status: 204, body: '' });
    });
    await page.route('**/api/me/parq', (route) => {
      if (route.request().method() === 'GET') return json(route, { questions: PARQ });
      parq.push(route.request().postDataJSON());
      return json(route, { flags_raised: 0 });
    });
    // The plan request is drafted in a new topic of the athlete's own thread.
    const conversation = {
      id: 'conv-plan-draft',
      title: 'Plan',
      model: 'gemini',
      total_tokens: 0,
      created_at: '2026-11-13T09:00:00Z',
      updated_at: '2026-11-13T09:00:00Z',
    };
    await page.route(`**/api/chat/conversations/${conversation.id}/messages**`, (route) => json(route, { messages: [] }));
    await page.route('**/api/chat/personal/topics', (route) =>
      json(route, { conversation: { ...conversation, personal: true }, divider: null }, 201),
    );
    await page.route(/\/api\/chat\/conversations(\?.*)?$/, (route, request) =>
      request.method() === 'POST'
        ? json(route, conversation, 201)
        : json(route, { conversations: [], total: 0, limit: 50, offset: 0 }),
    );
    await page.goto('/');
    await signInThroughHostedPage(page);
    await expect(page.getByTestId('home-page')).toBeVisible({ timeout: 15_000 });
    return { writes, parq };
  }

  test('« Build my plan » opens the health check first while the PAR-Q is put off, then drafts the plan', async ({ page }) => {
    const { writes, parq } = await onboardedWith(page, [
      { step_id: 'parq', status: 'skipped' },
      { step_id: 'coach_proposal', status: 'complete' },
    ]);
    const empty = page.getByTestId('home-today-panel').getByTestId('home-plan-empty');
    await expect(empty).toContainText('The health check first, then your agent lays out your season, week by week.');

    await empty.getByRole('button', { name: 'Build my plan' }).click();
    const sheet = page.getByRole('dialog', { name: 'Health check' });
    await expect(sheet).toBeVisible();
    await expect(sheet.getByText(PARQ[6].text)).toBeVisible();
    await sheet.getByRole('button', { name: 'None of these apply' }).click();

    await expect(sheet).toHaveCount(0);
    expect(parq).toEqual([{ answers: PARQ.map((q) => ({ id: q.id, yes: false })) }]);
    expect(writes).toContainEqual({ step_id: 'parq', status: 'complete' });
    // Answered, the plan request is drafted in the conversation.
    await expect(page.getByPlaceholder('Message Dravr...').first()).toHaveValue(
      'Build me a training plan for my goal race.',
    );
  });

  test('closing the health-check sheet answers nothing, and the plan still waits', async ({ page }) => {
    const { writes } = await onboardedWith(page, [
      { step_id: 'parq', status: 'skipped' },
      { step_id: 'coach_proposal', status: 'complete' },
    ]);
    const empty = page.getByTestId('home-today-panel').getByTestId('home-plan-empty');
    await empty.getByRole('button', { name: 'Build my plan' }).click();
    const sheet = page.getByRole('dialog', { name: 'Health check' });
    await sheet.getByRole('button', { name: 'Close' }).click();
    await expect(sheet).toHaveCount(0);
    expect(writes).toEqual([]);
    await empty.getByRole('button', { name: 'Build my plan' }).click();
    await expect(page.getByRole('dialog', { name: 'Health check' })).toBeVisible();
  });
});
