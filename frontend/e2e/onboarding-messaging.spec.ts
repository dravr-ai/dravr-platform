// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: E2E for the chat app outside the wizard (carnet#1525) — Start lands on Home, then a Getting started row links Telegram
// ABOUTME: A coach's group thread offers the chat app once, as the agent's card; « Not now » is stored and holds across reloads

import { test, expect, type Page, type Route } from '@playwright/test';
import { mockHomeFirstArrival, setupDashboardMocks, signInThroughHostedPage } from './test-helpers';

const USER_ID = 'user-123';

const json = (route: Route, body: unknown, status = 200) =>
  route.fulfill({ status, contentType: 'application/json', body: JSON.stringify(body) });

const CHANNELS = [
  { channel: 'telegram', display_name: 'Telegram', method: 'deep_link', recommended: true },
  { channel: 'whatsapp', display_name: 'WhatsApp', method: 'deep_link', recommended: false },
];

/**
 * The workspace offers Telegram and WhatsApp; nothing is linked until
 * `completeLink()`. Every onboarding step write is recorded and accepted.
 */
async function installChatAppMocks(page: Page) {
  const state = { linked: false, stepWrites: [] as string[] };
  await page.route('**/api/messaging/channels/available', (route) => json(route, CHANNELS));
  await page.route('**/api/messaging/link/init/**', (route) => {
    const channel = route.request().url().split('/').pop() ?? 'telegram';
    return json(route, {
      channel,
      method: 'deep_link',
      code: 'abc123',
      linking_url: `https://t.me/DravrBot?start=abc123`,
      expires_at: '2030-01-01T00:00:00Z',
      qr_svg: '<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"></svg>',
    });
  });
  await page.route('**/api/messaging/links', (route) =>
    json(route, {
      links: state.linked
        ? [{ channel: 'telegram', channel_user_id: 'tg1', display_name: null, linked_at: '2026-11-12T10:00:00Z' }]
        : [],
    }),
  );
  await page.route('**/api/me/onboarding/steps/**', async (route) => {
    const step = route.request().url().split('/').pop();
    state.stepWrites.push(`${step}:${(route.request().postDataJSON() as { status: string }).status}`);
    await route.fulfill({ status: 204, body: '' });
  });
  return {
    state,
    completeLink() {
      state.linked = true;
    },
  };
}

test.describe('the athlete', () => {
  /** A connected athlete whose questions are answered: the wizard opens on Your team. */
  async function reachTeamStep(page: Page) {
    await page.addInitScript((userId) => {
      for (const flag of ['profile_type_chosen', 'about_you_done', 'parq_done', 'check_in_done']) {
        window.localStorage.setItem(`dravr.${flag}.${userId}`, '1');
      }
    }, USER_ID);
    await setupDashboardMocks(page, { role: 'user', email: 'lea@test.com', displayName: 'Léa' });
    await mockHomeFirstArrival(page);
    const mocks = await installChatAppMocks(page);
    // Registered last, so it wins: connected, Your team not answered yet.
    await page.route('**/api/me/onboarding-status', (route) =>
      json(route, { needs_provider_connection: false, steps: [], chosen_channel: null, coaches_others: false }),
    );
    const lead = { agent: { id: 'agent-endurance', title: 'Endurance', category: 'training' }, match_score: 0.9, reason: 'You run.' };
    await page.route('**/api/agents/proposal', (route) =>
      json(route, {
        profile: { has_profile: false, window_days: 28, total_activities: 0, primary_sport: null, sport_mix: [] },
        agents: [lead],
        team: { lead, members: [] },
      }),
    );
    await page.route('**/api/agents/*/usage', (route) => route.fulfill({ status: 204, body: '' }));
    await page.route('**/api/chat/personal/topics', (route) =>
      json(
        route,
        {
          conversation: {
            id: 'conv-home',
            title: 'Endurance',
            agent_id: 'agent-endurance',
            personal: true,
            total_tokens: 0,
            created_at: '2026-11-12T10:00:00Z',
            updated_at: '2026-11-12T10:00:00Z',
          },
          divider: null,
        },
        201,
      ),
    );
    await page.goto('/');
    await signInThroughHostedPage(page);
    await expect(page.getByRole('button', { name: 'Start with my team · 1' })).toBeVisible({ timeout: 15_000 });
    return mocks;
  }

  test('Start with my team lands on Home, never on a chat-app step, though the workspace offers apps', async ({
    page,
  }) => {
    await reachTeamStep(page);
    await page.getByRole('button', { name: 'Start with my team · 1' }).click();

    await expect(page.getByTestId('onboarding-flow')).toHaveCount(0, { timeout: 15_000 });
    await expect(page).toHaveURL(/#home\//);
    // The two screens that used to follow are gone from the journey.
    await expect(page.getByText(/Almost there/i)).toHaveCount(0);
    await expect(page.getByText(/Where do you want to chat with your agent/i)).toHaveCount(0);
  });

  test('Getting started offers the chat app, opens its sheet, links Telegram and ticks the row', async ({ page }) => {
    await setupDashboardMocks(page, { role: 'user', email: 'lea@test.com', displayName: 'Léa' });
    await mockHomeFirstArrival(page);
    const mocks = await installChatAppMocks(page);
    const row = (done: boolean) => ({
      id: 'chat_app',
      order: 6,
      eligible: true,
      done,
      current: false,
      postback: null,
    });
    await page.route('**/api/me/getting-started', (route) =>
      json(route, {
        rows: [
          { id: 'connect', order: 1, eligible: true, done: true, current: false, postback: null },
          { id: 'meet_team', order: 2, eligible: true, done: false, current: true, postback: 'gs:2:meet_team' },
          { id: 'last_workout', order: 3, eligible: true, done: false, current: false, postback: 'gs:3:last_workout' },
          { id: 'about_me_more', order: 4, eligible: true, done: false, current: false, postback: 'gs:4:about_me_more' },
          { id: 'check_in', order: 5, eligible: true, done: false, current: false, postback: 'gs:5:check_in' },
          row(mocks.state.linked),
        ],
        done_count: mocks.state.linked ? 2 : 1,
        total: 6,
        folded_at: null,
        hidden: false,
      }),
    );

    await page.goto('/');
    await signInThroughHostedPage(page);

    const chatApp = page.getByTestId('home-getting-started-row-chat_app');
    await expect(chatApp).toContainText('Message me elsewhere too', { timeout: 15_000 });
    await expect(chatApp).toContainText('Telegram or WhatsApp');
    await chatApp.getByRole('button').click();

    const sheet = page.getByRole('dialog', { name: 'Pick your app' });
    await expect(sheet).toBeVisible();
    await sheet.getByTestId('chat-app-choose-telegram').click();
    await expect(page.getByRole('dialog', { name: 'Connect Telegram' })).toBeVisible();
    // On a desktop the QR reaches the phone; the open button is the other way.
    await expect(page.getByRole('img', { name: /QR code to connect Telegram/i })).toBeVisible();
    await expect(page.getByRole('button', { name: 'Open Telegram' })).toBeVisible();
    await expect(page.getByTestId('chat-app-use-another')).toBeVisible();

    // The link lands on the phone: the sheet closes, says so, and the row ticks.
    mocks.completeLink();
    await expect(page.getByTestId('chat-app-sheet')).toHaveCount(0, { timeout: 15_000 });
    await expect(page.getByText('Telegram is set up')).toBeVisible();
    await expect(chatApp).toHaveAttribute('data-state', 'done', { timeout: 15_000 });
    await expect(chatApp).toContainText('Telegram set up');
    expect(mocks.state.stepWrites).toContain('messaging_configure:complete');
  });
});

test.describe('the coach', () => {
  /** A coach past onboarding who owns one group, with its thread on the list. */
  async function installCoachMocks(page: Page, steps: { step_id: string; status: string }[] = []) {
    await setupDashboardMocks(page, { role: 'user', email: 'coach@test.com', displayName: 'Philippe' });
    const mocks = await installChatAppMocks(page);
    const answered = { steps: [{ step_id: 'coach_proposal', status: 'complete' }, { step_id: 'coach_group', status: 'complete' }, ...steps] };
    await page.route('**/api/me/onboarding-status', (route) =>
      json(route, {
        needs_provider_connection: false,
        steps: [
          ...answered.steps,
          ...mocks.state.stepWrites
            .filter((write) => write.startsWith('messaging_configure:'))
            .map((write) => ({ step_id: 'messaging_configure', status: write.split(':')[1] })),
        ],
        chosen_channel: null,
        coaches_others: true,
      }),
    );
    const thread = {
      id: 'c-1',
      title: 'Léa · Marathon de Montréal',
      agent_id: 'agent-endurance',
      agent_title: 'Endurance',
      group_id: 'g-1',
      group_name: 'Léa · Marathon de Montréal',
      message_count: 0,
      unread_count: 0,
      created_at: '2026-11-12T09:00:00Z',
      updated_at: '2026-11-12T09:00:00Z',
      last_message: null,
    };
    await page.route(/\/api\/chat\/conversations(\?.*)?$/, (route) =>
      json(route, { conversations: [thread], total: 1, limit: 50, offset: 0 }),
    );
    await page.route('**/api/chat/conversations/c-1/**', (route) => {
      const path = new URL(route.request().url()).pathname;
      if (path.endsWith('/messages')) return json(route, { messages: [], feedback: [] });
      if (path.endsWith('/verdicts')) return json(route, { verdicts: [] });
      if (path.endsWith('/participants')) return json(route, { participants: [] });
      return route.fulfill({ status: 204, body: '' });
    });
    await page.route('**/api/groups/g-1', (route) =>
      json(route, {
        id: 'g-1',
        tenant_id: USER_ID,
        name: 'Léa · Marathon de Montréal',
        description: null,
        agent_id: 'agent-endurance',
        agent_title: 'Endurance',
        agent_handle: 'endurance',
        owner_id: USER_ID,
        coach_user_id: USER_ID,
        coach_display_name: 'Philippe',
        peer_data_sharing: false,
        respond_mode: 'all',
        digest_mode: 'off',
        max_members: 10,
        is_active: true,
        created_at: '2026-11-12T09:00:00Z',
        updated_at: '2026-11-12T09:00:00Z',
      }),
    );
    await page.route('**/api/groups/g-1/transcript**', (route) => json(route, { entries: [] }));
    return mocks;
  }

  /** The coach, signed in, in the group thread of the group they own. */
  async function openOwnGroup(page: Page, steps: { step_id: string; status: string }[] = []) {
    const mocks = await installCoachMocks(page, steps);
    await page.goto('/#chat/c-1');
    await signInThroughHostedPage(page);
    return mocks;
  }

  test('the group’s agent offers the chat app once, and « Not now » holds after a reload', async ({ page }) => {
    const mocks = await openOwnGroup(page);

    const card = page.getByTestId('group-chat-app-offer');
    await expect(card).toContainText('Write to me from Telegram too?', { timeout: 15_000 });
    await card.getByRole('button', { name: 'Not now' }).click();

    await expect(card).toHaveCount(0, { timeout: 10_000 });
    expect(mocks.state.stepWrites).toEqual(['messaging_configure:skipped']);

    // The answer is the server's: a fresh session does not ask again.
    await page.reload();
    await signInThroughHostedPage(page, { mockHostedPage: false });
    await page.evaluate(() => {
      window.location.hash = '#chat/c-1';
    });
    await expect(page.getByTestId('conversation-header-title')).toBeVisible({ timeout: 15_000 });
    await expect(page.getByTestId('group-chat-app-offer')).toHaveCount(0);
  });

  test('Set up Telegram opens the same sheet, straight on Telegram', async ({ page }) => {
    await openOwnGroup(page);
    await page.getByRole('button', { name: 'Set up Telegram' }).click({ timeout: 15_000 });
    const sheet = page.getByRole('dialog', { name: 'Connect Telegram' });
    await expect(sheet).toBeVisible();
    await expect(sheet.getByRole('button', { name: 'Open Telegram' })).toBeVisible();
    await sheet.getByTestId('chat-app-use-another').click();
    await expect(page.getByRole('dialog', { name: 'Pick your app' })).toBeVisible();
  });

  // The P3·A arrival frame: the wizard's last button lands in the new group's
  // thread — no chat-app screen in between, though the workspace offers two —
  // and the agent's card is waiting there.
  test('Go to my group lands in the new group’s thread, where the agent offers the chat app', async ({ page }) => {
    await page.addInitScript((userId) => {
      window.localStorage.setItem(`dravr.profile_type_chosen.${userId}`, '1');
      window.localStorage.setItem(`dravr.coach_proposal_done.${userId}`, '1');
      window.localStorage.setItem(`dravr.coach_seat_done.${userId}`, '1');
    }, USER_ID);
    const mocks = await installCoachMocks(page);
    let groupMade = false;
    await page.route('**/api/me/onboarding-status', (route) =>
      json(route, {
        needs_provider_connection: false,
        steps: mocks.state.stepWrites.map((write) => {
          const [step_id, status] = write.split(':');
          return { step_id, status };
        }),
        chosen_channel: null,
        coaches_others: true,
      }),
    );
    await page.route('**/api/agents**', async (route) => {
      if (route.request().method() !== 'GET' || new URL(route.request().url()).pathname !== '/api/agents') {
        return route.fallback();
      }
      await json(route, {
        agents: [{ id: 'agent-endurance', title: 'Endurance', description: null, category: 'training', tags: [] }],
        total: 1,
        metadata: { timestamp: new Date().toISOString(), api_version: 'v1' },
      });
    });
    await page.route('**/api/groups', async (route) => {
      groupMade = true;
      // A coach still without the seat: an active one leaves the invite for
      // the coach Home instead of the group's thread.
      await json(route, { id: 'g-1', tenant_id: USER_ID, name: 'Léa', description: null, agent_id: 'agent-endurance', owner_id: USER_ID, coach_user_id: null }, 201);
    });
    await page.route('**/api/groups/g-1/invites', (route) =>
      json(route, { id: 'i-1', group_id: 'g-1', code: 'ABCD2345', kind: 'member' }, 201),
    );
    await page.route(/\/api\/chat\/conversations(\?.*)?$/, async (route) => {
      const thread = {
        id: 'c-1',
        title: 'Léa',
        agent_id: 'agent-endurance',
        agent_title: 'Endurance',
        group_id: 'g-1',
        group_name: 'Léa',
        message_count: 0,
        unread_count: 0,
        created_at: '2026-11-12T09:00:00Z',
        updated_at: '2026-11-12T09:00:00Z',
        last_message: null,
      };
      if (route.request().method() === 'POST') return json(route, thread, 201);
      const conversations = groupMade ? [thread] : [];
      await json(route, { conversations, total: conversations.length, limit: 50, offset: 0 });
    });
    await page.goto('/');
    await signInThroughHostedPage(page);

    // The group step is one screen: its name, the agent defaulted, Create;
    // then the invite, whose one way on is « Go to my group ».
    await page.getByTestId('onboarding-group-name').fill('Léa', { timeout: 15_000 });
    await expect(page.getByTestId('onboarding-group-agent')).toContainText('Endurance');
    await page.getByRole('button', { name: 'Create the group' }).click();
    await expect(page.getByTestId('onboarding-group-link')).toContainText('/groups/join/ABCD2345');
    await page.getByTestId('onboarding-group-done').click();

    await expect(page.getByTestId('onboarding-flow')).toHaveCount(0, { timeout: 15_000 });
    await expect(page).toHaveURL(/#chat\/c-1$/);
    await expect(page.getByText(/Where do you want to chat with your agent/i)).toHaveCount(0);
    await expect(page.getByTestId('group-chat-app-offer')).toContainText('Write to me from Telegram too?', {
      timeout: 15_000,
    });
  });

  test('a coach who already answered is not offered the chat app', async ({ page }) => {
    await openOwnGroup(page, [{ step_id: 'messaging_configure', status: 'skipped' }]);
    await expect(page.getByTestId('conversation-header-title')).toBeVisible({ timeout: 15_000 });
    await expect(page.getByTestId('group-chat-app-offer')).toHaveCount(0);
  });
});
