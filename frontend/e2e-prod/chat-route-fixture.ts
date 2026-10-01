// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: A chat thread whose coach reply is one short line over the latest run's route card, for the production-build map specs
// ABOUTME: Answers the conversation list, its messages and verdicts the way the server does; nothing reaches the network

import type { Page } from '@playwright/test';
import { ROUTE } from './home-map-fixture';

const CONVERSATION_ID = 'conv-route';
export const CHAT_ROUTE_TITLE = 'Morning trail';
export const CHAT_ROUTE_QUESTION = 'Show me my run';

/**
 * Serve one conversation: the athlete's question and a coach reply that is a
 * single short sentence with the route under it. A short sentence is the case
 * that matters — the card once shrank to the width of the words above it.
 */
export async function mockChatRoute(page: Page): Promise<void> {
  await page.route('**/api/chat/conversations/*/verdicts', (route) =>
    route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({ verdicts: [], total: 0 }),
    }),
  );
  await page.route('**/api/chat/conversations/*/messages', async (route, request) => {
    if (request.method() !== 'GET') {
      await route.fallback();
      return;
    }
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({
        messages: [
          {
            id: 'msg-route-user',
            conversation_id: CONVERSATION_ID,
            role: 'user',
            content: CHAT_ROUTE_QUESTION,
            created_at: '2026-09-30T16:00:00Z',
          },
          {
            id: 'msg-route-coach',
            conversation_id: CONVERSATION_ID,
            role: 'assistant',
            content: 'Here it is.\n\n⟦viz:0⟧',
            created_at: '2026-09-30T16:00:05Z',
            scene_blocks: JSON.stringify([{ kind: 'route', ...ROUTE }]),
          },
        ],
      }),
    });
  });
  await page.route(/\/api\/chat\/conversations(\?.*)?$/, async (route, request) => {
    if (request.method() !== 'GET') {
      await route.fallback();
      return;
    }
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({
        conversations: [
          {
            id: CONVERSATION_ID,
            title: CHAT_ROUTE_TITLE,
            created_at: '2026-09-30T15:59:00Z',
            updated_at: '2026-09-30T16:00:05Z',
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
