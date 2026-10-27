// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: A long turn on the web (carnet#1193) — a reload mid-turn restores the indicator and polls for the reply, and a finished turn folds its steps above the reply
// ABOUTME: Mocked API; screenshots the indicator and the folded line in both themes and checks their ink against the bubble

import { test, expect, type Page } from '@playwright/test';
import { setupDashboardMocks, loginToDashboard } from './test-helpers';

const CONVERSATION_ID = 'conv-trail';
const QUESTION = 'Build Léa’s plan for the spring';
const REPLY = 'Léa’s spring plan is saved: twelve weeks, three phases.';

const CONVERSATIONS = {
  conversations: [
    {
      id: CONVERSATION_ID,
      title: 'Plan for Léa',
      agent_id: 'coach-endurance',
      agent_name: 'Endurance Agent',
      created_at: '2026-10-26T09:00:00Z',
      updated_at: '2026-10-26T09:06:00Z',
      message_count: 2,
      unread_count: 0,
    },
  ],
  total: 1,
  limit: 50,
  offset: 0,
};

const EARLIER = [
  { id: 'm1', conversation_id: CONVERSATION_ID, role: 'user', content: 'Hi', created_at: '2026-10-26T09:00:00Z' },
  {
    id: 'm2',
    conversation_id: CONVERSATION_ID,
    role: 'assistant',
    content: 'Hi, ready when you are.',
    created_at: '2026-10-26T09:00:10Z',
  },
];

async function mockConversationList(page: Page) {
  await page.route('**/api/chat/conversations**', async (route, request) => {
    const url = request.url();
    if (url.includes('/messages') || url.includes('/read') || url.includes('/stop')) {
      await route.fallback();
      return;
    }
    await route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify(CONVERSATIONS) });
  });
}

async function eachTheme(page: Page, shoot: (theme: 'light' | 'dark') => Promise<void>) {
  for (const theme of ['light', 'dark'] as const) {
    await page.evaluate(dark => document.documentElement.classList.toggle('dark', dark), theme === 'dark');
    // The palette swap re-runs every colour transition on screen; sampling
    // before it lands reads a mid-transition ink.
    await page.waitForTimeout(1500);
    await shoot(theme);
  }
  await page.evaluate(() => document.documentElement.classList.remove('dark'));
}

/** The element's text ink, as the browser resolved it. */
async function ink(page: Page, testId: string) {
  return page.getByTestId(testId).first().evaluate(el => getComputedStyle(el).color);
}

test.describe('a long turn', () => {
  test('a reload mid-turn shows the reply as coming, then the reply when the poll finds it', async ({ page }, testInfo) => {
    const askedAt = new Date(Date.now() - 134_000).toISOString();
    const question = { id: 'q1', conversation_id: CONVERSATION_ID, role: 'user', content: QUESTION, created_at: askedAt };
    let answered = false;
    let reads = 0;

    await setupDashboardMocks(page, { role: 'user' });
    await page.route('**/api/chat/conversations/*/messages**', async (route, request) => {
      if (request.method() !== 'GET') {
        await route.fallback();
        return;
      }
      reads += 1;
      const body = answered
        ? {
            messages: [
              ...EARLIER,
              question,
              { id: 'r1', conversation_id: CONVERSATION_ID, role: 'assistant', content: REPLY, created_at: new Date().toISOString() },
            ],
            awaiting_reply: null,
          }
        : { messages: [...EARLIER, question], awaiting_reply: { question_id: 'q1', asked_at: askedAt } };
      await route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify(body) });
    });
    await mockConversationList(page);
    await loginToDashboard(page);
    await page.goto(`/#chat/${CONVERSATION_ID}`);

    const indicator = page.getByTestId('turn-indicator');
    await expect(indicator).toBeVisible();
    await expect(page.getByTestId('turn-current-step')).toContainText('Thinking');
    await expect(page.getByTestId('turn-elapsed')).toContainText(/2 min \d+ s/);
    await expect(page.getByTestId('turn-safe-to-leave')).toBeVisible();
    await expect(page.getByTestId('stop-turn-button')).toBeVisible();

    await eachTheme(page, async theme => {
      const step = await ink(page, 'turn-current-step');
      const note = await ink(page, 'turn-safe-to-leave');
      expect(step, `${theme}: the step is inked`).not.toBe('rgba(0, 0, 0, 0)');
      expect(note, `${theme}: the note is inked`).not.toBe('rgba(0, 0, 0, 0)');
      await page.screenshot({ path: testInfo.outputPath(`restored-indicator-${theme}.png`) });
    });

    // The page keeps reading the thread until the reply is there.
    const readsBefore = reads;
    answered = true;
    await expect(page.locator('[data-role="assistant"]', { hasText: REPLY })).toBeVisible({ timeout: 10_000 });
    expect(reads).toBeGreaterThan(readsBefore);
    await expect(indicator).toHaveCount(0);
    await expect(page.getByTestId('stop-turn-button')).toHaveCount(0);

    // ...and stops once it is.
    const readsAfter = reads;
    await page.waitForTimeout(9_000);
    expect(reads).toBe(readsAfter);
  });

  test('a finished turn folds its steps into one line above the reply', async ({ page }, testInfo) => {
    await setupDashboardMocks(page, { role: 'user' });
    const frames = [
      ['progress', { kind: 'stage', id: 'prompt_assembly', title: 'prompt_assembly', status: 'started' }],
      ['progress', { kind: 'tool', id: 'c1', title: 'get_group_member_activities', status: 'InProgress' }],
      ['progress', { kind: 'tool', id: 'c1', title: 'get_group_member_activities', status: 'Completed' }],
      ['progress', { kind: 'tool', id: 'c2', title: 'recommend_plan_flavour', status: 'InProgress' }],
      ['progress', { kind: 'tool', id: 'c2', title: 'recommend_plan_flavour', status: 'Completed' }],
      ['progress', { kind: 'tool', id: 'c3', title: 'save_training_plan', status: 'Completed' }],
      ['delta', { delta: REPLY }],
      [
        'done',
        {
          turn_id: 'turn-1',
          user_message: { id: 'm3', role: 'user', content: QUESTION, created_at: '2026-10-26T09:07:00Z' },
          assistant: {
            message: { id: 'm4', role: 'assistant', content: REPLY, created_at: '2026-10-26T09:15:00Z' },
            blocks: [{ type: 'prose', text: REPLY }],
            finish_reason: 'stop',
          },
          conversation_updated_at: '2026-10-26T09:15:00Z',
          telemetry: {
            model: 'm',
            provider_name: 'p',
            tool_calls_count: 3,
            tools_called: ['get_group_member_activities', 'recommend_plan_flavour', 'save_training_plan'],
            execution_time_ms: 480_000,
          },
        },
      ],
    ] as const;
    const body = frames.map(([event, data]) => `event: ${event}\ndata: ${JSON.stringify(data)}\n\n`).join('');
    await page.route('**/api/chat/conversations/*/messages**', async (route, request) => {
      if (request.method() === 'POST') {
        await route.fulfill({ status: 200, contentType: 'text/event-stream', body });
        return;
      }
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({ messages: EARLIER, awaiting_reply: null }),
      });
    });
    await mockConversationList(page);
    await loginToDashboard(page);
    await page.goto(`/#chat/${CONVERSATION_ID}`);
    await expect(page.getByTestId('message-row').first()).toBeVisible();

    await page.getByPlaceholder('Message Dravr...').first().fill(QUESTION);
    await page.getByRole('button', { name: 'Send message' }).click();

    const reply = page.locator('[data-role="assistant"]', { hasText: REPLY });
    await expect(reply).toBeVisible();
    const folded = reply.getByTestId('turn-trail-folded');
    const toggle = folded.getByRole('button');
    await expect(toggle).toContainText('3 steps');
    await toggle.click();
    const steps = folded.getByTestId('turn-trail-steps');
    await expect(steps).toContainText('Reading a member’s activities');
    await expect(steps).toContainText('Choosing the training approach');
    await expect(steps).toContainText('Saving the plan');
    await expect(page.getByText(/get_group_member_activities|recommend_plan_flavour|save_training_plan/)).toHaveCount(0);

    await eachTheme(page, async theme => {
      await page.screenshot({ path: testInfo.outputPath(`folded-trail-${theme}.png`) });
    });
  });
});
