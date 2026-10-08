// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: E2E for carnet#735 — « Start » on the onboarding proposal lands in a thread the agent has already opened
// ABOUTME: The welcome renders under the agent's name with its starters as buttons; tapping one sends its postback as the next turn

import { test, expect, type Page, type Route } from '@playwright/test';
import { setupDashboardMocks, signInThroughHostedPage } from './test-helpers';

const USER_ID = 'user-123';
const AGENT_ID = 'agent-fuelling';
const AGENT_TITLE = 'Fuelling Agent';
const CONVERSATION_ID = 'conv-welcome';
/**
 * The starters the server writes for an athlete who has just connected a
 * provider and told Dravr nothing yet (carnet#828): a catalogue starter ranked
 * from that state takes the first slot, the agent's own examples fill the
 * rest. Each button posts an opaque postback the server resolves; the athlete
 * only ever sees the label.
 */
const STARTERS = [
  { label: 'Get to know me', postback: 'uc:0:about_me', resolved: '/pillars' },
  { label: 'What should I eat before a 6am run?', postback: 'ex:1:0', resolved: 'What should I eat before a 6am run?' },
  { label: 'How do I carb load for a marathon?', postback: 'ex:2:1', resolved: 'How do I carb load for a marathon?' },
];

const json = (route: Route, body: unknown, status = 200) =>
  route.fulfill({ status, contentType: 'application/json', body: JSON.stringify(body) });

/** The row the server writes when the thread is created with the agent. */
const welcome = {
  id: 'welcome-1',
  role: 'assistant',
  content: "Hi! Dravr's Fuelling Agent here.\n\nFuelling specialist for endurance athletes.",
  finish_reason: 'agent_welcome',
  created_at: '2026-10-02T10:00:00Z',
  actions: {
    title: 'To get started, you can ask me:',
    actions: STARTERS.map((s) => ({ label: s.label, action_type: 'postback', value: s.postback })),
  },
};

/**
 * A first-run athlete who has just connected a provider: the onboarding status
 * reads "needs a provider" until `connect()`, then not — the in-session
 * transition the proposal step waits for.
 */
async function installMocks(page: Page) {
  const state = { needsProvider: true, created: false, sentTurns: [] as string[] };

  await page.route('**/api/me/onboarding-status', (route) =>
    json(route, { needs_provider_connection: state.needsProvider }),
  );
  await page.route('**/api/me/onboarding/steps/**', (route) => route.fulfill({ status: 204, body: '' }));
  await page.route('**/api/messaging/channels/available', (route) => json(route, []));
  await page.route('**/api/agents/proposal', (route) =>
    json(route, {
      profile: {
        has_profile: false,
        window_days: 28,
        total_activities: 0,
        primary_sport: null,
        sport_mix: [],
      },
      agents: [
        {
          agent: { id: AGENT_ID, title: AGENT_TITLE, category: 'nutrition' },
          match_score: 0.9,
          reason: 'You run long and fuel little.',
        },
      ],
    }),
  );
  await page.route('**/api/agents/*/usage', (route) => route.fulfill({ status: 204, body: '' }));

  await page.route('**/api/chat/conversations**', async (route) => {
    const request = route.request();
    const url = new URL(request.url());
    if (request.method() === 'POST' && url.pathname === '/api/chat/conversations') {
      state.created = true;
      return json(
        route,
        {
          id: CONVERSATION_ID,
          title: AGENT_TITLE,
          model: 'gemini',
          agent_id: AGENT_ID,
          total_tokens: 0,
          created_at: '2026-10-02T10:00:00Z',
          updated_at: '2026-10-02T10:00:00Z',
        },
        201,
      );
    }
    if (url.pathname === `/api/chat/conversations/${CONVERSATION_ID}/messages`) {
      if (request.method() === 'POST') {
        const sent = (request.postDataJSON() as { content: string }).content;
        state.sentTurns.push(sent);
        return json(route, {
          turn_id: 'turn-1',
          // The server stores what a postback resolves to, never the postback.
          user_message: {
            id: 'u1',
            role: 'user',
            content: STARTERS.find((s) => s.postback === sent)?.resolved ?? sent,
            created_at: '2026-10-02T10:01:00Z',
          },
          assistant: {
            message: { id: 'a1', role: 'assistant', content: 'Oatmeal, 2 hours out.', created_at: '2026-10-02T10:01:01Z' },
            blocks: [{ type: 'prose', text: 'Oatmeal, 2 hours out.' }],
            finish_reason: 'stop',
          },
          conversation_updated_at: '2026-10-02T10:01:01Z',
          telemetry: { model: 'gemini', provider_name: 'gemini', tool_calls_count: 0, tools_called: [], execution_time_ms: 10 },
        });
      }
      return json(route, { messages: state.created ? [welcome] : [], feedback: [] });
    }
    if (url.pathname.endsWith('/read')) return route.fulfill({ status: 204, body: '' });
    if (url.pathname.endsWith('/verdicts')) return json(route, { verdicts: [] });
    if (url.pathname.endsWith('/participants')) return json(route, { participants: [] });
    if (url.pathname === '/api/chat/conversations') {
      return json(route, {
        conversations: state.created
          ? [
              {
                id: CONVERSATION_ID,
                title: AGENT_TITLE,
                model: 'gemini',
                message_count: 1,
                total_tokens: 0,
                agent_id: AGENT_ID,
                agent_title: AGENT_TITLE,
                unread_count: 0,
                created_at: '2026-10-02T10:00:00Z',
                updated_at: '2026-10-02T10:00:00Z',
              },
            ]
          : [],
        total: state.created ? 1 : 0,
        limit: 50,
        offset: 0,
      });
    }
    return route.fallback();
  });

  return {
    state,
    connect() {
      state.needsProvider = false;
    },
  };
}

test('« Start » on the proposal opens a thread the agent has already welcomed', async ({ page }) => {
  await page.addInitScript((userId) => {
    window.localStorage.setItem(`dravr.profile_type_chosen.${userId}`, '1');
    window.localStorage.setItem(`dravr.about_you_done.${userId}`, '1');
    window.localStorage.setItem(`dravr.parq_done.${userId}`, '1');
  }, USER_ID);
  await setupDashboardMocks(page, { role: 'user', email: 'fresh@test.com', displayName: 'Fresh User' });
  const mocks = await installMocks(page);

  await page.goto('/');
  await signInThroughHostedPage(page);
  await expect(page.getByRole('heading', { name: /welcome, fresh user/i })).toBeVisible();

  // The provider lands: the status flips once its cache is stale and the tab
  // regains focus, which is what moves the flow to the agent proposal.
  mocks.connect();
  await page.waitForTimeout(5_500);
  await page.evaluate(() => {
    window.dispatchEvent(new Event('visibilitychange'));
    window.dispatchEvent(new Event('focus'));
  });

  await expect(page.getByRole('heading', { name: AGENT_TITLE })).toBeVisible({ timeout: 15_000 });
  await page.getByRole('button', { name: 'Start' }).click();

  await expect(page.getByText(/Dravr's Fuelling Agent here/)).toBeVisible({ timeout: 15_000 });
  await expect(page.getByText('To get started, you can ask me:')).toBeVisible();
  for (const s of STARTERS) {
    await expect(page.getByRole('button', { name: s.label })).toBeVisible();
  }

  // A tap posts the starter's postback, not its label.
  await page.getByRole('button', { name: STARTERS[0].label }).click();
  await expect.poll(() => mocks.state.sentTurns).toEqual([STARTERS[0].postback]);
  await expect(page.getByText(/\b(uc|ex):\d/)).toHaveCount(0);
});
