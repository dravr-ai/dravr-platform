// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The verdict drawer at phone width on a touch screen — pills wrap inside it, taps reach the preview
// ABOUTME: A touch screen has no hover, so a tap must pin the source preview open and a second tap close it

import { test, expect, type Locator, type Page } from '@playwright/test';
import { setupDashboardMocks, loginToDashboard, openPersonalConversation } from './test-helpers';

const CONVERSATION_ID = 'conv-mobile-verdict';
// Long enough that the pill has to truncate at 393px rather than overflow.
const CONVERSATION_TITLE = 'Sortie facile de mardi après le bloc de seuil de septembre';
const MESSAGE_ID = 'msg-mobile-verdict';
const CLAIM = 'Si tu t’es senti fluide : tu peux considérer la séance comme bonne.';
const REPLY = [
  'Je n’ai pas tes zones de FC enregistrées, donc c’est une **estimation**.',
  '',
  '- Si tu voulais un vrai footing facile : vise 5 à 10 bpm de moins.',
  `- ${CLAIM}`,
].join('\n');

async function setupMocks(page: Page) {
  await setupDashboardMocks(page, { role: 'user', email: 'alice@acme.com', displayName: 'Alice Test' });

  await page.route('**/api/chat/conversations/*/verdicts', async (route) => {
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({
        verdicts: [
          {
            id: 'verdict-mobile-1',
            conversation_id: CONVERSATION_ID,
            message_id: MESSAGE_ID,
            agent_id: null,
            claim_text: `- ${CLAIM}`,
            category: 'training_prescription',
            status: 'supported',
            evidence_strength: 'mixed',
            confidence: 0.8,
            layer_fired: 'evidence',
            explanation:
              'Supported by Rønnestad and Mujika 2014 (online 2013), Optimizing strength training for running and cycling endurance performance: A review, Scand J Med Sci Sports',
            evidence_refs: 'doi:10.1111/sms.12104,pmid:22389869',
            created_at: '2026-10-05T13:52:00Z',
          },
        ],
        total: 1,
      }),
    });
  });

  await page.route('**/api/chat/conversations/*/messages', async (route, request) => {
    if (request.method() !== 'GET') return route.fallback();
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({
        messages: [
          {
            id: 'msg-mobile-user',
            conversation_id: CONVERSATION_ID,
            role: 'user',
            content: 'Comment était ma sortie ?',
            created_at: '2026-10-05T13:51:00Z',
          },
          {
            id: MESSAGE_ID,
            conversation_id: CONVERSATION_ID,
            role: 'assistant',
            content: REPLY,
            created_at: '2026-10-05T13:52:00Z',
          },
        ],
      }),
    });
  });

  await page.route(/\/api\/chat\/conversations(\?.*)?$/, async (route, request) => {
    if (request.method() !== 'GET') return route.fallback();
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({
        conversations: [
          {
            id: CONVERSATION_ID,
            title: CONVERSATION_TITLE,
            agent_id: null,
            created_at: '2026-10-05T13:50:00Z',
            updated_at: '2026-10-05T13:52:00Z',
            message_count: 2,
          },
        ],
        total: 1,
        limit: 50,
        offset: 0,
      }),
    });
  });
}

/** Open the athlete's own conversation from Home's History, and the drawer from its reply's chip. */
async function openDrawer(page: Page): Promise<Locator> {
  await openPersonalConversation(page, CONVERSATION_TITLE);
  await expect(page.getByText('Comment était ma sortie ?')).toBeVisible({ timeout: 10000 });
  await page.getByRole('button', { name: /1 verdict · supported/ }).tap();
  const drawer = page.getByTestId('verdict-drawer');
  await expect(drawer).toBeVisible();
  return drawer;
}

/** Fail when `inner` reaches past `outer` on either side. */
async function expectInside(inner: Locator, outer: Locator) {
  const a = await inner.boundingBox();
  const b = await outer.boundingBox();
  expect(a && b).toBeTruthy();
  if (!a || !b) return;
  expect(a.x).toBeGreaterThanOrEqual(b.x - 0.5);
  expect(a.x + a.width).toBeLessThanOrEqual(b.x + b.width + 0.5);
}

test.describe('Verdict drawer at phone width', () => {
  test.beforeEach(async ({ page }) => {
    await setupMocks(page);
    await loginToDashboard(page);
  });

  test('fills the screen, wraps its pills inside it, and keeps touch-sized targets', async ({ page }) => {
    const drawer = await openDrawer(page);

    // Below the drawer's max width it takes the whole screen.
    const viewport = page.viewportSize();
    const drawerBox = await drawer.boundingBox();
    expect(viewport && drawerBox).toBeTruthy();
    expect(drawerBox?.width ?? 0).toBeGreaterThanOrEqual((viewport?.width ?? Number.POSITIVE_INFINITY) - 0.5);

    const overflow = await page.evaluate(
      () => document.documentElement.scrollWidth - document.documentElement.clientWidth,
    );
    expect(overflow).toBeLessThanOrEqual(0);

    const pills = drawer.getByTestId('verdict-pills');
    await expectInside(pills, drawer);
    const targets = [
      drawer.getByRole('link', { name: 'Study 1' }),
      drawer.getByRole('link', { name: 'Study 2' }),
      drawer.getByTestId('verdict-source-pill'),
    ];
    for (const target of targets) {
      await expectInside(target, drawer);
      const box = await target.boundingBox();
      expect(box?.height ?? 0).toBeGreaterThanOrEqual(44);
    }

    // The long title is cut with an ellipsis rather than pushing the pill wide.
    const title = drawer.getByTestId('verdict-source-pill').locator('span');
    const truncated = await title.evaluate((el) => el.scrollWidth > el.clientWidth);
    expect(truncated).toBe(true);
  });

  test('a tap pins the source preview open inside the drawer, a second tap closes it', async ({ page }) => {
    const drawer = await openDrawer(page);
    const pill = drawer.getByTestId('verdict-source-pill');
    const preview = drawer.getByTestId('verdict-source-preview');

    await expect(preview).toBeHidden();
    await pill.tap();
    await expect(preview).toBeVisible();
    await expect(preview.locator('mark')).toHaveText(CLAIM);
    await expectInside(preview, drawer);

    await pill.tap();
    await expect(preview).toBeHidden();
  });

  test('the actions menu opens on a tap and stays on screen', async ({ page }) => {
    const drawer = await openDrawer(page);
    const trigger = drawer.getByRole('button', { name: 'Verdict actions' });
    const box = await trigger.boundingBox();
    expect(box?.height ?? 0).toBeGreaterThanOrEqual(44);

    await trigger.tap();
    const menu = page.getByRole('menu', { name: 'Verdict actions' });
    await expect(menu).toBeVisible();
    await expectInside(menu, drawer);
  });
});
