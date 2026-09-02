// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Playwright E2E tests for the web chat surface profile and the typed TurnEnvelope.
// ABOUTME: Covers the coach-authored assistant turn, markdown/plan/scene rendering, and one verdict affordance.

import { test, expect, type Page } from '@playwright/test';
import { setupDashboardMocks, loginToDashboard } from './test-helpers';

const CONVERSATION_ID = 'conv-coached';
const CONVERSATION_TITLE = 'Bloc seuil de septembre';
const COACH_ID = 'coach-tempo';
const COACH_TITLE = 'Coach Tempo';
const ASSISTANT_MESSAGE_ID = 'msg-assistant-1';

/**
 * The English caveat banner header (`EN_VERIFICATION_WARN_SUFFIX`). Web is a
 * chip surface, so this string must never reach a web reply — its presence
 * beside the chips is the double-warning the TurnEnvelope work removed.
 */
const CAVEAT_BANNER_HEADER = "A few claims I couldn't formally back up";

/** The flagged sentence. It belongs to the reply exactly once. */
const FLAGGED_CLAIM = 'Your VO2max is 82.';

const ASSISTANT_REPLY = [
  'That block looks **solid**.',
  '',
  FLAGGED_CLAIM,
  '',
  'Keep the easy days easy.',
].join('\n');

/** A resolved chart, index-aligned with the reply prose marker. */
const SCENE_CHART = {
  kind: 'chart',
  view_box: { width: 640, height: 360 },
  nodes: [
    { node: 'line', x1: 40, y1: 320, x2: 600, y2: 320, stroke: 'axis', width: 1 },
    { node: 'path', d: 'M40 300 L200 240 L360 180 L520 120', stroke: 'activity', width: 2 },
    {
      node: 'text',
      x: 40,
      y: 340,
      content: 'Week 1',
      anchor: 'start',
      baseline: 'hanging',
      role: 'axis_tick',
      color: 'label',
    },
  ],
  legend: [{ label: 'Weekly TSS', color: 'activity' }],
  title: 'Weekly load, last four weeks',
  source_tool: 'get_activities',
};

const WORKOUT_PLAN_BLOCK = {
  kind: 'workout_plan',
  source_tool: 'save_training_plan',
  plan: {
    plan_window: { start: '2026-09-01', end: '2026-09-07' },
    rationale: 'One threshold session, everything else aerobic.',
    compliance: { z1_pct: 70, z2_pct: 20, z3_pct: 10, weekly_tss_target: 420 },
    evidence_refs: [],
    weeks: [
      {
        week_index: 1,
        days: [
          {
            day: 'Tue',
            session: {
              name: 'Seuil 3x10',
              duration_min: 55,
              intensity_factor: 0.88,
              tss_estimate: 72,
              blocks: [],
            },
          },
          { day: 'Wed', session: null },
        ],
      },
    ],
  },
};

interface ChatMockOptions {
  /** Serve the assistant turn with a resolved chart and its ⟦viz:0⟧ marker. */
  withScene?: boolean;
  /** Serve the assistant turn with a workout-plan card block. */
  withWorkoutPlan?: boolean;
  /** Serve a claim verdict attached to the assistant message. */
  withVerdict?: boolean;
  /** Report the athlete as approaching the daily message quota. */
  usageWarning?: boolean;
}

function assistantContent(withScene: boolean): string {
  return withScene ? `${ASSISTANT_REPLY}\n\n⟦viz:0⟧` : ASSISTANT_REPLY;
}

async function setupChatMocks(page: Page, options: ChatMockOptions = {}) {
  const {
    withScene = false,
    withWorkoutPlan = false,
    withVerdict = false,
    usageWarning = false,
  } = options;

  await setupDashboardMocks(page, { role: 'user' });

  const sceneBlocks: unknown[] = [];
  if (withScene) sceneBlocks.push(SCENE_CHART);
  if (withWorkoutPlan) sceneBlocks.push(WORKOUT_PLAN_BLOCK);

  const assistantMessage = {
    id: ASSISTANT_MESSAGE_ID,
    conversation_id: CONVERSATION_ID,
    role: 'assistant',
    content: assistantContent(withScene),
    created_at: '2026-08-20T10:01:00Z',
    ...(sceneBlocks.length > 0 ? { scene_blocks: JSON.stringify(sceneBlocks) } : {}),
  };

  await page.route('**/api/chat/conversations/*/verdicts', async (route) => {
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({
        verdicts: withVerdict
          ? [
              {
                id: 'verdict-1',
                conversation_id: CONVERSATION_ID,
                message_id: ASSISTANT_MESSAGE_ID,
                coach_id: COACH_ID,
                claim_text: FLAGGED_CLAIM,
                category: 'physiological',
                status: 'contradicted',
                evidence_strength: 'none',
                confidence: 0.91,
                layer_fired: 'deterministic',
                explanation: 'Outside the plausible range for the athlete.',
                evidence_refs: null,
                created_at: '2026-08-20T10:01:02Z',
              },
            ]
          : [],
        total: withVerdict ? 1 : 0,
      }),
    });
  });

  await page.route('**/api/chat/conversations/*/messages', async (route, request) => {
    if (request.method() === 'GET') {
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({
          messages: [
            {
              id: 'msg-user-1',
              conversation_id: CONVERSATION_ID,
              role: 'user',
              content: 'How did the block go?',
              created_at: '2026-08-20T10:00:00Z',
            },
            assistantMessage,
          ],
        }),
      });
      return;
    }
    await route.fallback();
  });

  await page.route(/\/api\/chat\/conversations(\?.*)?$/, async (route, request) => {
    if (request.method() === 'GET') {
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({
          conversations: [
            {
              id: CONVERSATION_ID,
              title: CONVERSATION_TITLE,
              coach_id: COACH_ID,
              coach_name: COACH_TITLE,
              created_at: '2026-08-20T09:00:00Z',
              updated_at: '2026-08-20T10:01:00Z',
              message_count: 2,
            },
          ],
          total: 1,
          limit: 50,
          offset: 0,
        }),
      });
      return;
    }
    await route.fallback();
  });

  await page.route(/\/api\/coaches(\?.*)?$/, async (route) => {
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({
        coaches: [
          {
            id: COACH_ID,
            title: COACH_TITLE,
            description: 'Threshold work and race-week sharpening',
            system_prompt: 'You are a tempo coach.',
            category: 'training',
            tags: [],
            token_count: 40,
            is_favorite: false,
            use_count: 7,
            last_used_at: '2026-08-20T10:01:00Z',
            created_at: '2026-07-01T00:00:00Z',
            updated_at: '2026-08-20T10:01:00Z',
            is_system: false,
            visibility: 'private',
            is_assigned: false,
          },
        ],
        total: 1,
      }),
    });
  });

  await page.route('**/api/providers', async (route) => {
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({
        providers: [
          {
            provider: 'strava',
            display_name: 'Strava',
            requires_oauth: true,
            connected: true,
            capabilities: ['activities', 'athlete'],
          },
        ],
      }),
    });
  });

  if (usageWarning) {
    const counter = (over: boolean) => ({
      allowed: true,
      current: over ? 45 : 0,
      limit: over ? 50 : 500000,
      warning: over,
      burst_zone: false,
      resets_at: '2026-08-21T00:00:00Z',
    });
    await page.route('**/api/usage/status', async (route) => {
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({
          daily: {
            messages: counter(true),
            tokens: counter(false),
            tool_calls: counter(false),
          },
          weekly: {
            messages: counter(false),
            tokens: counter(false),
            tool_calls: counter(false),
          },
          resources: { coaches: 1, max_coaches: 3, conversations: 1, max_conversations: 20 },
        }),
      });
    });
  }
}

/** Open the coached conversation from the sidebar and wait for its turn. */
async function openConversation(page: Page) {
  await expect(page.getByText(CONVERSATION_TITLE)).toBeVisible({ timeout: 10000 });
  await page.getByText(CONVERSATION_TITLE).click();
  await expect(page.getByText('How did the block go?')).toBeVisible({ timeout: 10000 });
}

/**
 * The assistant turn, anchored on its OWN author avatar rather than on any
 * text that also appears in the conversation header (whose icon carries an
 * empty alt precisely so it cannot be confused for an author).
 */
function assistantTurn(page: Page) {
  return page
    .locator('[data-testid="message-row"][data-role="assistant"]')
    .filter({ has: page.locator(`img[alt="${COACH_TITLE}"]`) });
}

test.describe('Chat - the assistant turn is authored by the coach', () => {
  test('an assistant turn in a coach conversation is authored by the coach title, not Dravr', async ({ page }) => {
    await setupChatMocks(page);
    await loginToDashboard(page);
    await openConversation(page);

    // The header renders the coach title too, so a bare getByText would pass
    // even with the bug present. The avatar's alt text is the author element.
    await expect(page.locator(`img[alt="${COACH_TITLE}"]`)).toHaveCount(1);
    await expect(page.locator('img[alt="Dravr"]')).toHaveCount(0);

    const turn = assistantTurn(page);
    await expect(turn).toHaveCount(1);
    await expect(turn).toContainText(COACH_TITLE);
    await expect(turn).toContainText('Keep the easy days easy.');
    await expect(turn).not.toContainText('Dravr');

    // The user's own turn is still labelled "You" — the coach label is scoped
    // to the assistant side, not applied to the whole thread.
    const userTurn = page
      .locator('[data-testid="message-row"][data-role="user"]')
      .filter({ hasText: 'How did the block go?' });
    await expect(userTurn.first()).toContainText('You');
  });
});

test.describe('Chat - the web surface profile', () => {
  test('prose renders as markdown, not as literal asterisks', async ({ page }) => {
    await setupChatMocks(page);
    await loginToDashboard(page);
    await openConversation(page);

    await expect(assistantTurn(page).locator('strong', { hasText: 'solid' })).toHaveCount(1);
    await expect(page.getByText('That block looks **solid**')).toHaveCount(0);
  });

  test('a workout plan renders as a card, not as raw plan JSON', async ({ page }) => {
    await setupChatMocks(page, { withWorkoutPlan: true });
    await loginToDashboard(page);
    await openConversation(page);

    const turn = assistantTurn(page);
    await expect(turn).toContainText('Training plan');
    await expect(turn).toContainText('2026-09-01 → 2026-09-07');
    await expect(turn).toContainText('Seuil 3x10');
    await expect(turn).toContainText('55 min · IF 0.88 · 72 TSS');
    await expect(turn).toContainText('Weekly TSS');
    // The card replaced the JSON; the plan's field names never reach the page.
    await expect(turn).not.toContainText('plan_window');
    await expect(turn).not.toContainText('intensity_factor');
  });

  test('a scene block renders inline as SVG', async ({ page }) => {
    await setupChatMocks(page, { withScene: true });
    await loginToDashboard(page);
    await openConversation(page);

    const turn = assistantTurn(page);
    const chart = turn.getByRole('img', { name: 'Chart: Weekly load, last four weeks' });
    await expect(chart).toHaveCount(1);
    // Inline SVG, drawn by the client from resolved primitives — not an <img>
    // pointing at a rasterized chart the way a media surface receives one.
    await expect(chart).toHaveJSProperty('tagName', 'svg');
    await expect(chart.locator('path')).toHaveCount(1);
    await expect(chart.locator('line')).toHaveCount(1);
    await expect(chart.locator('css=text')).toHaveText('Week 1');
    await expect(turn).toContainText('Weekly load, last four weeks');
    await expect(turn).toContainText('source: get_activities');
    // The positional marker is consumed by the renderer, never shown.
    await expect(turn).not.toContainText('⟦viz:0⟧');
  });
});

test.describe('Chat - one verdict affordance per surface', () => {
  test('a flagged claim renders verdict chips and no caveat banner', async ({ page }) => {
    await setupChatMocks(page, { withVerdict: true });
    await loginToDashboard(page);
    await openConversation(page);

    const turn = assistantTurn(page);

    // Half one: the chips are present, and they say what was flagged — the
    // qualifier is the verdict's status word; its evidence strength stays in the drawer.
    const chip = turn.getByRole('button', { name: /1 verdict · contradicted/ });
    await expect(chip).toHaveCount(1);
    await expect(turn.getByRole('button', { name: 'Ask me about this claim' })).toHaveCount(1);

    // Half two: the reply itself is untouched. No banner header, no bullet
    // echo of the claim, and the flagged sentence appears exactly once.
    await expect(turn).not.toContainText(CAVEAT_BANNER_HEADER);
    await expect(turn).not.toContainText(`- ${FLAGGED_CLAIM}`);
    const occurrences = await turn.evaluate(
      (node, claim) => (node.textContent ?? '').split(claim).length - 1,
      FLAGGED_CLAIM,
    );
    expect(occurrences).toBe(1);
  });

  test('a turn with no flagged claim shows no chips at all', async ({ page }) => {
    await setupChatMocks(page);
    await loginToDashboard(page);
    await openConversation(page);

    const turn = assistantTurn(page);
    await expect(turn).toContainText('Keep the easy days easy.');
    await expect(turn.getByRole('button', { name: /verdict/ })).toHaveCount(0);
    await expect(turn).not.toContainText(CAVEAT_BANNER_HEADER);
  });
});

test.describe('Chat - the turn envelope carries no usage headers', () => {
  test('the usage warning comes from /api/usage/status and a header-free turn still renders', async ({ page }) => {
    await setupChatMocks(page, { usageWarning: true });

    // Count the client's own usage lookups: the banner must have a source of its
    // own now that the turn response carries no X-Usage-* headers.
    let usageStatusRequests = 0;
    page.on('request', (request) => {
      if (request.url().includes('/api/usage/status')) {
        usageStatusRequests += 1;
      }
    });
    // A finished turn, exactly as the wire now carries it: a typed block list,
    // no X-Usage-* headers, no is_command_response, no card_title.
    await page.route('**/api/chat/conversations/*/messages', async (route, request) => {
      if (request.method() !== 'POST') {
        await route.fallback();
        return;
      }
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({
          turn_id: '00000000-0000-4000-8000-000000000042',
          user_message: {
            id: 'msg-user-2',
            conversation_id: CONVERSATION_ID,
            role: 'user',
            content: 'And next week?',
            created_at: '2026-08-20T10:05:00Z',
          },
          assistant: {
            message: {
              id: 'msg-assistant-2',
              conversation_id: CONVERSATION_ID,
              role: 'assistant',
              content: 'Next week we hold the same threshold dose.',
              created_at: '2026-08-20T10:05:02Z',
            },
            blocks: [{ type: 'prose', text: 'Next week we hold the same threshold dose.' }],
            finish_reason: 'stop',
          },
          conversation_updated_at: '2026-08-20T10:05:02Z',
          telemetry: {
            model: 'gemini-1.5-flash',
            provider_name: 'gemini',
            tool_calls_count: 0,
            tools_called: [],
            execution_time_ms: 900,
          },
        }),
      });
    });

    await loginToDashboard(page);
    await openConversation(page);

    // The banner is already up before any turn is sent: its source is the
    // usage-status endpoint, not a header on a chat response.
    const banner = page.getByTestId('usage-warning-banner');
    await expect(banner).toBeVisible({ timeout: 10000 });
    await expect(banner).toContainText("You've used 90% of your daily messages (45/50)");

    const input = page.getByPlaceholder('Message Dravr...');
    await input.click();
    await input.pressSequentially('And next week?', { delay: 10 });
    const sendBtn = page.getByRole('button', { name: 'Send message' });
    await expect(sendBtn).toBeEnabled({ timeout: 5000 });
    await sendBtn.click();

    // The header-free turn painted normally and nothing errored.
    await expect(page.getByText('Next week we hold the same threshold dose.')).toBeVisible({
      timeout: 10000,
    });
    await expect(page.getByText(/HTTP error/i)).toHaveCount(0);

    // Asserting the mock's own headers would prove nothing -- the mock sets none.
    // What is worth pinning is that the client never asked for them: the usage
    // banner above came from /api/usage/status, so the turn response carrying no
    // X-Usage-* header changed nothing the athlete sees.
    expect(usageStatusRequests).toBeGreaterThan(0);

    // The banner survives the turn, still sourced from the status endpoint.
    await expect(banner).toContainText("You've used 90% of your daily messages (45/50)");
  });
});

/**
 * One turn as the server's single stream delivers it: the stages the pipeline
 * worked through, any `delta` frames, each reply block, then the `done` frame
 * carrying the same envelope the blocking branch returns.
 */
function sseTurn(
  turn: { assistant: { blocks: unknown[] } },
  deltas: string[] = [],
  progress: unknown[] = [],
): string {
  return [
    ...progress.map((p) => `event: progress\ndata: ${JSON.stringify(p)}\n\n`),
    ...deltas.map((delta) => `event: delta\ndata: ${JSON.stringify({ delta })}\n\n`),
    ...turn.assistant.blocks.map((block) => `event: block\ndata: ${JSON.stringify(block)}\n\n`),
    `event: done\ndata: ${JSON.stringify(turn)}\n\n`,
  ].join('');
}

/** The stage sequence a real turn emits, in pipeline order. */
const STAGE_PROGRESS = [
  { kind: 'stage', id: 'prompt_assembly', title: 'prompt_assembly', status: 'started' },
  { kind: 'stage', id: 'prompt_assembly', title: 'prompt_assembly', status: 'finished' },
  { kind: 'stage', id: 'dispatch', title: 'dispatch', status: 'started' },
];

const NEXT_WEEK_REPLY = 'Next week we hold the same threshold dose.';

/** A finished turn, shaped exactly as `turn_response.rs` serializes it. */
function turnEnvelope(options: { content?: string; finishReason?: string } = {}) {
  const { content = NEXT_WEEK_REPLY, finishReason = 'stop' } = options;
  return {
    turn_id: '00000000-0000-4000-8000-000000000077',
    user_message: {
      id: `msg-user-${finishReason}`,
      conversation_id: CONVERSATION_ID,
      role: 'user',
      content: 'And next week?',
      created_at: '2026-08-24T10:05:00Z',
    },
    assistant: {
      message: {
        id: `msg-assistant-${finishReason}`,
        conversation_id: CONVERSATION_ID,
        role: 'assistant',
        content,
        created_at: '2026-08-24T10:05:02Z',
      },
      blocks: [{ type: 'prose', text: content }],
      finish_reason: finishReason,
    },
    conversation_updated_at: '2026-08-24T10:05:02Z',
    telemetry: {
      model: 'gpt-5-codex',
      provider_name: 'copilot',
      tool_calls_count: 0,
      tools_called: [],
      execution_time_ms: 900,
    },
  };
}

/** Type into the composer and press Send. */
async function composeAndSend(page: Page, text: string) {
  const input = page.getByPlaceholder('Message Dravr...');
  await input.click();
  await input.pressSequentially(text, { delay: 10 });
  const sendBtn = page.getByRole('button', { name: 'Send message' });
  await expect(sendBtn).toBeEnabled({ timeout: 5000 });
  await sendBtn.click();
}

test.describe('Chat - one transport for every turn', () => {
  test('the web send goes out on the shared sendTurn request and renders the SSE reply', async ({
    page,
  }) => {
    // Regression this turns red: the hand-rolled fetch coming back. That one
    // set its own headers, so `X-Client-Platform` never reached the server on
    // a chat turn and the surface could not tell web from mobile.
    await setupChatMocks(page);

    const sent: { headers: Record<string, string>; body: string }[] = [];
    await page.route('**/api/chat/conversations/*/messages', async (route, request) => {
      if (request.method() !== 'POST') {
        await route.fallback();
        return;
      }
      sent.push({ headers: request.headers(), body: request.postData() ?? '' });
      await route.fulfill({
        status: 200,
        contentType: 'text/event-stream',
        body: sseTurn(turnEnvelope()),
      });
    });

    await loginToDashboard(page);
    await openConversation(page);
    await composeAndSend(page, 'And next week?');

    await expect(page.getByText(NEXT_WEEK_REPLY)).toBeVisible({ timeout: 10000 });

    expect(sent).toHaveLength(1);
    expect(sent[0].headers['x-client-platform']).toBe('web');
    expect(sent[0].headers['accept']).toBe('text/event-stream, application/json');
    const body = JSON.parse(sent[0].body) as Record<string, unknown>;
    expect(body.content).toContain('And next week?');
    // Everything the turn reports rides the response body, so the request has
    // nothing to correlate and carries nothing but the message.
    expect(Object.keys(body)).toEqual(['content']);
  });

  test('the reply is drawn from the block frames, not from the envelope list', async ({
    page,
  }) => {
    // The one stream carries each renderable piece as its own frame. This
    // sends a turn whose `done` envelope lists NO blocks at all and puts them
    // only in the frames — so a client that ignored the frames and walked the
    // envelope draws nothing, and this fails.
    await setupChatMocks(page);

    const turn = turnEnvelope();
    const framedBlocks = [
      { type: 'prose', text: NEXT_WEEK_REPLY },
      {
        type: 'actions',
        title: 'Pick a session',
        actions: [
          { label: 'Seuil 3x10', action_type: 'postback', value: '/plan session seuil' },
        ],
      },
    ];

    await page.route('**/api/chat/conversations/*/messages', async (route, request) => {
      if (request.method() !== 'POST') {
        await route.fallback();
        return;
      }
      await route.fulfill({
        status: 200,
        contentType: 'text/event-stream',
        body: [
          ...STAGE_PROGRESS.map((p) => `event: progress\ndata: ${JSON.stringify(p)}\n\n`),
          ...framedBlocks.map((b) => `event: block\ndata: ${JSON.stringify(b)}\n\n`),
          `event: done\ndata: ${JSON.stringify({
            ...turn,
            assistant: { ...turn.assistant, blocks: [] },
          })}\n\n`,
        ].join(''),
      });
    });

    await loginToDashboard(page);
    await openConversation(page);
    await composeAndSend(page, 'And next week?');

    await expect(page.getByText(NEXT_WEEK_REPLY)).toBeVisible({ timeout: 10000 });
    // The controls came off a `block` frame the client consumed as it arrived.
    await expect(page.getByRole('button', { name: 'Seuil 3x10' })).toBeVisible({
      timeout: 10000,
    });
  });

  test('no client ever opens the deleted AG-UI stream', async ({ page }) => {
    // The route is gone server-side. This pins the other half: nothing in the
    // client still reaches for it, so a stale hook reintroduced by a merge
    // fails here rather than 404-ing silently in production.
    await setupChatMocks(page);

    const aguiRequests: string[] = [];
    page.on('request', (request) => {
      if (request.url().includes('/api/agui/')) aguiRequests.push(request.url());
    });

    await page.route('**/api/chat/conversations/*/messages', async (route, request) => {
      if (request.method() !== 'POST') {
        await route.fallback();
        return;
      }
      await route.fulfill({
        status: 200,
        contentType: 'text/event-stream',
        body: sseTurn(turnEnvelope(), [], STAGE_PROGRESS),
      });
    });

    await loginToDashboard(page);
    await openConversation(page);
    await composeAndSend(page, 'And next week?');
    await expect(page.getByText(NEXT_WEEK_REPLY)).toBeVisible({ timeout: 10000 });

    expect(aguiRequests).toEqual([]);
  });

  test('a slash-command answer arrives as one JSON document and renders without a content-type sniff', async ({
    page,
  }) => {
    // Regression this turns red: reintroducing the content-type branch, or
    // funnelling commands down a second send path. The command answers before
    // the streaming branch is chosen, so the body is a bare envelope — and the
    // same parser must read it.
    await setupChatMocks(page);
    await page.route('**/api/chat/conversations/*/messages', async (route, request) => {
      if (request.method() !== 'POST') {
        await route.fallback();
        return;
      }
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify(turnEnvelope({ content: 'Coach Tempo is active.', finishReason: 'command' })),
      });
    });

    await loginToDashboard(page);
    await openConversation(page);
    await composeAndSend(page, '/coach select coach-tempo');

    await expect(page.getByText('Coach Tempo is active.')).toBeVisible({ timeout: 10000 });
    await expect(page.getByText(/HTTP error/i)).toHaveCount(0);
  });

  test('an error frame surfaces the server message instead of an empty turn', async ({ page }) => {
    await setupChatMocks(page);
    await page.route('**/api/chat/conversations/*/messages', async (route, request) => {
      if (request.method() !== 'POST') {
        await route.fallback();
        return;
      }
      await route.fulfill({
        status: 200,
        contentType: 'text/event-stream',
        body: 'event: failed\ndata: {"error":"Daily message limit reached."}\n\n',
      });
    });

    await loginToDashboard(page);
    await openConversation(page);
    await composeAndSend(page, 'And next week?');

    await expect(page.getByText('Daily message limit reached.')).toBeVisible({ timeout: 10000 });
  });

  test('retry drops the answered turn and re-sends the question, with no composer round-trip', async ({
    page,
  }) => {
    // Regression this turns red: the simulated `[aria-label="Send message"]`
    // click coming back. That path seeded the composer, so after a retry the
    // box still held the question — and it depended on a DOM node that any
    // markup change could rename out from under it.
    await setupChatMocks(page);

    const sent: string[] = [];
    await page.route('**/api/chat/conversations/*/messages', async (route, request) => {
      if (request.method() !== 'POST') {
        await route.fallback();
        return;
      }
      const body = JSON.parse(request.postData() ?? '{}') as { content: string };
      sent.push(body.content);
      await route.fulfill({
        status: 200,
        contentType: 'text/event-stream',
        body: sseTurn(
          turnEnvelope({
            content: sent.length === 1 ? NEXT_WEEK_REPLY : 'On second thought: hold the volume.',
          }),
        ),
      });
    });

    await loginToDashboard(page);
    await openConversation(page);
    await composeAndSend(page, 'And next week?');
    await expect(page.getByText(NEXT_WEEK_REPLY)).toBeVisible({ timeout: 10000 });

    await page.getByTitle('Regenerate response').last().click();

    await expect(page.getByText('On second thought: hold the volume.')).toBeVisible({
      timeout: 10000,
    });
    // The retried turn replaced the one it retried rather than stacking on it.
    await expect(page.getByText(NEXT_WEEK_REPLY)).toHaveCount(0);
    // The question went back out verbatim, and the composer stayed empty.
    expect(sent).toHaveLength(2);
    expect(sent[1]).toContain('And next week?');
    await expect(page.getByPlaceholder('Message Dravr...')).toHaveValue('');
  });
});

test.describe('Chat - the client paints the block, it does not scrape the prose', () => {
  const RECONNECT_URL =
    'https://www.strava.com/oauth/authorize?client_id=1&scope=activity:read_all';

  test('the reconnect CTA comes off the block, with no URL left in the reply text', async ({
    page,
  }) => {
    // Regression this turns red: the deleted
    // `/https?:\/\/\S*\/providers\/sciotte\/login\?token=\S+/` scrape coming
    // back. The prose here carries NO url — on a surface that renders a
    // reconnect control the server does not fold the sentence in — so a
    // regex-driven renderer draws no button and this fails.
    await setupChatMocks(page);
    const turn = turnEnvelope({ content: 'I need you to reconnect before I can read that.' });
    await page.route('**/api/chat/conversations/*/messages', async (route, request) => {
      if (request.method() !== 'POST') {
        await route.fallback();
        return;
      }
      await route.fulfill({
        status: 200,
        contentType: 'text/event-stream',
        body: sseTurn({
          ...turn,
          assistant: {
            ...turn.assistant,
            blocks: [
              { type: 'prose', text: 'I need you to reconnect before I can read that.' },
              {
                type: 'reconnect',
                provider: 'strava',
                display_name: 'Strava',
                url: RECONNECT_URL,
                text: 'Reconnect Strava to continue.',
              },
            ],
          },
        }),
      });
    });

    await loginToDashboard(page);
    await openConversation(page);
    await composeAndSend(page, 'How did last week go?');

    const cta = page.getByRole('link', { name: /Reconnect Strava/ });
    await expect(cta).toBeVisible({ timeout: 10000 });
    await expect(cta).toHaveAttribute('href', RECONNECT_URL);
    // The one-time URL is never printed as text beside the control.
    await expect(page.getByText(RECONNECT_URL)).toHaveCount(0);
  });

  test('a url action opens an allowed domain and refuses a foreign one', async ({ page }) => {
    // Phase 0 deleted an open-redirect: any URL in a reply was opened. This is
    // that affordance re-implemented from a server-declared block, gated on a
    // trusted-domain allowlist. Widening the allowlist to "anything" fails the
    // second half; deleting the arm fails the first.
    await setupChatMocks(page);
    const foreign = 'https://strava.com.attacker.example/steal';
    const turn = turnEnvelope({ content: 'Pick where to connect.' });
    await page.route('**/api/chat/conversations/*/messages', async (route, request) => {
      if (request.method() !== 'POST') {
        await route.fallback();
        return;
      }
      await route.fulfill({
        status: 200,
        contentType: 'text/event-stream',
        body: sseTurn({
          ...turn,
          assistant: {
            ...turn.assistant,
            blocks: [
              { type: 'prose', text: 'Pick where to connect.' },
              {
                type: 'actions',
                title: 'Connect a provider',
                actions: [
                  { label: 'Authorize Strava', action_type: 'url', value: RECONNECT_URL },
                  { label: 'Authorize elsewhere', action_type: 'url', value: foreign },
                ],
              },
            ],
          },
        }),
      });
    });
    // The popup must never actually navigate to a third party from a test.
    await page.context().route('**/oauth/authorize**', (route) =>
      route.fulfill({ status: 200, contentType: 'text/html', body: '<html>authorize</html>' }),
    );

    await loginToDashboard(page);
    await openConversation(page);
    await composeAndSend(page, 'Connect me.');

    await expect(page.getByRole('button', { name: 'Authorize Strava' })).toBeVisible({
      timeout: 10000,
    });

    const opened = page.context().waitForEvent('page', { timeout: 10000 });
    await page.getByRole('button', { name: 'Authorize Strava' }).click();
    const popup = await opened;
    expect(popup.url()).toBe(RECONNECT_URL);
    await popup.close();

    // The foreign host opens nothing at all.
    const pagesBefore = page.context().pages().length;
    await page.getByRole('button', { name: 'Authorize elsewhere' }).click();
    await page.waitForTimeout(1500);
    expect(page.context().pages().length).toBe(pagesBefore);
  });

  test('the usage banner states the counters the turn itself measured', async ({ page }) => {
    // Regression this turns red: the deleted `errorMessage.match(/in (\d+)
    // seconds/)` countdown coming back. That read a number out of an English
    // sentence; the notice block carries the counter, the cap and the reset.
    await setupChatMocks(page);
    const turn = turnEnvelope({ content: 'Here is your week.' });
    await page.route('**/api/chat/conversations/*/messages', async (route, request) => {
      if (request.method() !== 'POST') {
        await route.fallback();
        return;
      }
      await route.fulfill({
        status: 200,
        contentType: 'text/event-stream',
        body: sseTurn({
          ...turn,
          assistant: {
            ...turn.assistant,
            blocks: [
              { type: 'prose', text: 'Here is your week.' },
              {
                type: 'notice',
                notice: {
                  kind: 'quota_warning',
                  level: 'approaching',
                  current: 45,
                  limit: 50,
                  resets_at: '2026-08-26T00:00:00Z',
                },
              },
            ],
          },
        }),
      });
    });

    await loginToDashboard(page);
    await openConversation(page);
    await composeAndSend(page, 'How was my week?');

    await expect(page.getByText('Here is your week.')).toBeVisible({ timeout: 10000 });
    const banner = page.getByTestId('usage-warning-banner');
    await expect(banner).toBeVisible({ timeout: 10000 });
    await expect(banner).toContainText('(45/50)');
    await expect(banner).toContainText('90%');
  });
});
