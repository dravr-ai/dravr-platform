// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: WCAG 2.1 AA coverage for the chat shell — the whole athlete product after the cutover
// ABOUTME: Colour contrast is ENABLED here; the three older a11y specs all disable that rule

import { test, expect, type Page } from '@playwright/test';
import AxeBuilder from '@axe-core/playwright';
import { setupDashboardMocks, loginToDashboard, APP_SHELL_TIMEOUT_MS } from '../test-helpers';

const CONVERSATION = {
  id: 'conv-a11y-1',
  title: 'Sunday long run',
  coach_id: null,
  created_at: '2026-08-20T10:00:00Z',
  updated_at: '2026-08-20T10:00:00Z',
  message_count: 2,
  unread_count: 0,
  last_message: {
    preview: 'How did the long run feel?',
    role: 'assistant',
    created_at: '2026-08-20T10:00:00Z',
  },
};

const MESSAGES = [
  {
    id: 'm1',
    role: 'user',
    content: 'Did 32k this morning, felt strong until 26.',
    created_at: '2026-08-20T10:00:00Z',
  },
  {
    id: 'm2',
    role: 'assistant',
    content: 'That fade at 26k is the interesting part. How was fuelling?',
    created_at: '2026-08-20T10:01:00Z',
  },
];

async function setupChat(page: Page) {
  await setupDashboardMocks(page, { role: 'user' });

  // The list route is /api/chat/conversations — setupDashboardMocks answers it
  // with an EMPTY list, so these registrations (LIFO: later wins) put a real
  // thread in front of the scanner. A scan of an empty list would pass
  // vacuously and tell us nothing about the surface athletes actually read.
  // Anchored regex, not a glob: `**/api/chat/conversations**` also swallows
  // every sub-path (/messages, /participants, /read) and the shell stalls.
  await page.route(/\/api\/chat\/conversations(\?.*)?$/, async (route) =>
    route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({
        conversations: [CONVERSATION],
        total: 1,
        limit: 50,
        offset: 0,
      }),
    }),
  );
  await page.route(`**/api/chat/conversations/${CONVERSATION.id}/messages`, async (route) =>
    route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({ messages: MESSAGES }),
    }),
  );
  await page.route(`**/api/chat/conversations/${CONVERSATION.id}/participants`, async (route) =>
    route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({
        participants: [
          {
            user_id: 'owner-1',
            role: 'owner',
            added_by: 'owner-1',
            added_at: '2026-08-20T10:00:00Z',
          },
        ],
      }),
    }),
  );
  await page.route(`**/api/chat/conversations/${CONVERSATION.id}/verdicts`, async (route) =>
    route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({ verdicts: [] }),
    }),
  );

  await loginToDashboard(page);
  await page.waitForSelector('main', { timeout: APP_SHELL_TIMEOUT_MS });
  // Prove the fixture rendered. Scanning an empty shell would pass without
  // ever having looked at a conversation.
  await page
    .getByText(CONVERSATION.title)
    .first()
    .waitFor({ state: 'visible', timeout: APP_SHELL_TIMEOUT_MS });
}

/**
 * The scan the three older specs do not run.
 *
 * `color-contrast` is deliberately NOT in a disableRules list here. Every other
 * a11y spec in this suite turns it off — "until UI design fixes are
 * implemented", written in April and never revisited — which means the Boreal
 * palette has never actually been measured against WCAG on any surface.
 */
async function scan(page: Page) {
  return new AxeBuilder({ page }).withTags(['wcag2a', 'wcag2aa', 'wcag21aa']).analyze();
}

function report(violations: Awaited<ReturnType<typeof scan>>['violations']) {
  return violations
    .map(
      (v) =>
        `${v.id} (${v.impact}): ${v.help}\n` +
        v.nodes
          .slice(0, 4)
          .map((n) => `    ${n.target.join(' ')}\n      ${n.failureSummary?.split('\n')[1] ?? ''}`)
          .join('\n'),
    )
    .join('\n\n');
}

test.describe('Chat shell accessibility', () => {
  test.beforeEach(async ({ page }) => {
    await setupChat(page);
  });

  test('the conversation list has no WCAG 2.1 AA violations', async ({ page }) => {
    const results = await scan(page);
    expect(report(results.violations)).toBe('');
  });

  test('an open thread has no WCAG 2.1 AA violations', async ({ page }) => {
    await page.getByText(CONVERSATION.title).first().click();
    await page.waitForTimeout(500);

    const results = await scan(page);
    expect(report(results.violations)).toBe('');
  });

  test('the shell exposes the landmarks a screen reader navigates by', async ({ page }) => {
    await expect(page.locator('main, [role="main"]')).toBeVisible();
    expect(await page.locator('nav, [role="navigation"]').count()).toBeGreaterThanOrEqual(1);
  });

  test('the message composer is reachable and labelled', async ({ page }) => {
    await page.getByText(CONVERSATION.title).first().click();
    await page.waitForTimeout(500);

    // The composer is the primary control of the whole product; it must have
    // an accessible name, not just a placeholder.
    const composer = page.getByRole('textbox').last();
    await expect(composer).toBeVisible();
    await composer.focus();
    await expect(composer).toBeFocused();
  });

  test('every control the shell renders has an accessible name', async ({ page }) => {
    const results = await new AxeBuilder({ page })
      .withRules(['button-name', 'link-name', 'aria-input-field-name', 'label'])
      .analyze();
    expect(report(results.violations)).toBe('');
  });

  test('touch targets meet the WCAG 2.5.8 minimum', async ({ page }) => {
    // The sign-in page failed this on the 20px password toggle; the chat shell
    // has far more small controls, so it is worth its own assertion.
    const results = await new AxeBuilder({ page }).withRules(['target-size']).analyze();
    expect(report(results.violations)).toBe('');
  });
});
