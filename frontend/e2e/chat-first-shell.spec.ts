// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Locks the chat-first shell of the Chat-First Cutover (2026-08-26) on web
// ABOUTME: Landing on chat, the retired Coach tab, the "+" menu, and the @handle mention autocomplete

import { test, expect, type Page } from '@playwright/test';
import { setupDashboardMocks, loginToDashboard } from './test-helpers';

const CONVERSATION = {
  id: 'conv-1',
  title: 'Sunday long run',
  coach_id: null,
  created_at: '2026-08-20T10:00:00Z',
  updated_at: '2026-08-20T10:00:00Z',
  message_count: 0,
};

/** Every conversation POST the shell made, in order. */
interface CreatedConversation {
  body: Record<string, unknown>;
}

async function setupShellMocks(page: Page): Promise<CreatedConversation[]> {
  const created: CreatedConversation[] = [];

  // Base dashboard mocks first: later routes take priority (LIFO).
  await setupDashboardMocks(page, { role: 'user' });

  await page.route('**/api/chat/conversations/conv-1/participants', async (route) => {
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({
        participants: [
          { user_id: 'owner-1', role: 'owner', added_by: 'owner-1', added_at: '2026-08-20T10:00:00Z' },
        ],
      }),
    });
  });

  await page.route('**/api/chat/conversations/conv-1/messages', async (route) => {
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({ messages: [] }),
    });
  });

  await page.route('**/api/chat/conversations/conv-1/verdicts', async (route) => {
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({ verdicts: [], total: 0 }),
    });
  });

  await page.route(/\/api\/chat\/conversations(\?.*)?$/, async (route, request) => {
    if (request.method() === 'POST') {
      created.push({ body: request.postDataJSON() });
      await route.fulfill({
        status: 201,
        contentType: 'application/json',
        body: JSON.stringify({
          ...CONVERSATION,
          id: 'conv-new',
          title: 'New Conversation',
        }),
      });
      return;
    }
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({ conversations: [CONVERSATION], total: 1, limit: 50, offset: 0 }),
    });
  });

  await page.route('**/api/commands**', async (route) => {
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({ commands: [] }),
    });
  });

  await page.route(/\/api\/groups(\?.*)?$/, async (route) => {
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({
        groups: [
          {
            id: 'group-1',
            name: 'Sunday Riders',
            description: null,
            coach_id: 'coach-tempo',
            member_count: 4,
            is_active: true,
            peer_data_sharing: true,
            my_role: 'member',
            created_at: '2026-08-01T00:00:00Z',
          },
        ],
      }),
    });
  });

  // The athlete's coach list: one addressable coach, one personal coach with no handle.
  await page.route(/\/api\/coaches(\?.*)?$/, async (route) => {
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({
        coaches: [
          {
            id: 'coach-recovery',
            title: 'Recovery Coach',
            description: 'Sleep and recovery',
            system_prompt: 'You are a recovery coach.',
            category: 'recovery',
            tags: [],
            token_count: 100,
            is_favorite: false,
            use_count: 2,
            last_used_at: null,
            created_at: '2026-01-01T00:00:00Z',
            updated_at: '2026-01-01T00:00:00Z',
            is_system: true,
            handle: 'recovery-coach',
          },
          {
            id: 'coach-own',
            title: 'My Custom Coach',
            description: null,
            system_prompt: 'You are my coach.',
            category: 'training',
            tags: [],
            token_count: 50,
            is_favorite: false,
            use_count: 0,
            last_used_at: null,
            created_at: '2026-01-01T00:00:00Z',
            updated_at: '2026-01-01T00:00:00Z',
            is_system: false,
          },
        ],
        total: 2,
      }),
    });
  });

  return created;
}

test.describe('Chat-first shell', () => {
  test('a regular user lands on chat, and the sidebar offers Discover but no Coaches tab', async ({ page }) => {
    await setupShellMocks(page);
    await loginToDashboard(page);
    await page.waitForSelector('aside', { timeout: 10000 });

    await expect(page).toHaveURL(/#chat$/);
    await expect(page.getByPlaceholder('Message Dravr...').first()).toBeVisible({ timeout: 10000 });

    const aside = page.locator('aside');
    await expect(aside.getByRole('button', { name: 'Chat' })).toBeVisible();
    await expect(aside.getByRole('button', { name: 'Discover', exact: true })).toBeVisible();
    await expect(aside.getByRole('button', { name: 'Groups' })).toBeVisible();
    await expect(aside.getByRole('button', { name: 'Coaches' })).toHaveCount(0);
  });

  test('a stale #my-coaches deep link lands on chat', async ({ page }) => {
    await setupShellMocks(page);
    await loginToDashboard(page);
    await page.waitForSelector('aside', { timeout: 10000 });

    await page.goto('/#my-coaches');
    await page.waitForSelector('aside', { timeout: 10000 });

    await expect(page.getByPlaceholder('Message Dravr...').first()).toBeVisible({ timeout: 10000 });
    await expect(page).toHaveURL(/#chat$/);
    await expect(page.getByText('custom AI personas')).toHaveCount(0);
  });

  test('the "+" offers a new chat and a new group chat, and starts a plain chat', async ({ page }) => {
    const created = await setupShellMocks(page);
    await loginToDashboard(page);
    await page.waitForSelector('aside', { timeout: 10000 });

    await page.getByRole('button', { name: 'New', exact: true }).click();
    const menu = page.getByRole('menu', { name: 'Start a conversation' });
    await expect(menu.getByRole('menuitem')).toHaveText(['New chat', 'New group chat']);

    await menu.getByRole('menuitem', { name: 'New chat' }).click();

    await expect.poll(() => created.length).toBe(1);
    expect(created[0].body).not.toHaveProperty('group_id');
    await expect(page).toHaveURL(/#chat\/conv-new$/);
  });

  test('"New group chat" lists the coaching groups and opens a group-scoped conversation', async ({ page }) => {
    const created = await setupShellMocks(page);
    await loginToDashboard(page);
    await page.waitForSelector('aside', { timeout: 10000 });

    await page.getByRole('button', { name: 'New', exact: true }).click();
    await page.getByRole('menuitem', { name: 'New group chat' }).click();

    const picker = page.getByRole('dialog');
    await expect(picker.getByText('New group chat')).toBeVisible();
    await picker.getByRole('button', { name: /Sunday Riders/ }).click();

    await expect.poll(() => created.length).toBe(1);
    expect(created[0].body).toMatchObject({
      title: 'Sunday Riders',
      coach_id: 'coach-tempo',
      group_id: 'group-1',
    });
    await expect(page).toHaveURL(/#chat\/conv-new$/);
  });

  test('with a conversation open, "+" also offers adding someone, which opens the participants control', async ({ page }) => {
    await setupShellMocks(page);
    await loginToDashboard(page);
    await page.waitForSelector('aside', { timeout: 10000 });

    await page.goto('/#chat/conv-1');
    await page.waitForSelector('aside', { timeout: 10000 });
    await expect(page.getByTestId('conversation-header-title')).toHaveText('Sunday long run');

    await page.getByRole('button', { name: 'New', exact: true }).click();
    const menu = page.getByRole('menu', { name: 'Start a conversation' });
    await expect(menu.getByRole('menuitem')).toHaveText([
      'New chat',
      'New group chat',
      'Add someone to this discussion',
    ]);

    await menu.getByRole('menuitem', { name: 'Add someone to this discussion' }).click();

    const participants = page.getByRole('dialog', { name: 'Conversation participants' });
    await expect(participants).toBeVisible();
    await expect(participants.getByLabel('User id to add')).toBeVisible();
    await expect(participants.getByRole('list', { name: 'Participant list' })).toContainText('owner-1 · owner');
  });

  test('typing @ in the composer offers the installed coaches by handle and inserts the handle', async ({ page }) => {
    await setupShellMocks(page);
    await loginToDashboard(page);
    await page.waitForSelector('aside', { timeout: 10000 });

    await page.goto('/#chat/conv-1');
    await page.waitForSelector('aside', { timeout: 10000 });

    const composer = page.getByPlaceholder('Message Dravr...');
    await composer.click();
    await composer.type('Hey @rec');

    const palette = page.getByTestId('mention-palette');
    await expect(palette).toBeVisible();
    // Only the addressable coach is offered; the handle-less personal coach is not.
    await expect(palette.getByRole('option')).toHaveCount(1);
    await expect(palette.getByText('@recovery-coach')).toBeVisible();
    await expect(palette.getByText('My Custom Coach')).toHaveCount(0);

    await page.getByTestId('mention-palette-option-recovery-coach').click();

    await expect(composer).toHaveValue('Hey @recovery-coach ');
    await expect(palette).toHaveCount(0);
  });
});
