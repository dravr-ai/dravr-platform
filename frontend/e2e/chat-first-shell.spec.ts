// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Locks the chat shell on web — one list, the "+" menu, the "/" button and the header drawer
// ABOUTME: Landing on chat, the retired Coach and Groups tabs, and the @handle mention autocomplete

import { test, expect, type Page } from '@playwright/test';
import { setupDashboardMocks, loginToDashboard } from './test-helpers';

const CONVERSATION = {
  id: 'conv-1',
  title: 'Sunday long run',
  coach_id: null,
  created_at: '2026-08-20T10:00:00Z',
  updated_at: '2026-08-20T10:00:00Z',
  message_count: 0,
  unread_count: 3,
  last_message: {
    preview: 'How did the long run feel?',
    role: 'assistant',
    created_at: '2026-08-20T10:00:00Z',
  },
};

/** Every conversation POST the shell made, in order. */
interface CreatedConversation {
  body: Record<string, unknown>;
}

/** Everything the shell put on the wire: conversations created, turns sent. */
interface ShellTraffic {
  created: CreatedConversation[];
  sent: string[];
}

async function setupShellMocks(page: Page): Promise<ShellTraffic> {
  const created: CreatedConversation[] = [];
  const sent: string[] = [];

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

  await page.route('**/api/chat/conversations/conv-1/messages', async (route, request) => {
    if (request.method() === 'POST') {
      sent.push((request.postDataJSON() as { content: string }).content);
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({
          turn_id: 'turn-1',
          user_message: {
            id: 'm1',
            role: 'user',
            content: '',
            created_at: '2026-08-20T10:01:00Z',
          },
          assistant: {
            message: {
              id: 'm2',
              role: 'assistant',
              content: 'Done.',
              created_at: '2026-08-20T10:01:01Z',
            },
            blocks: [],
            finish_reason: 'command',
          },
          conversation_updated_at: '2026-08-20T10:01:01Z',
          telemetry: {
            model: 'command',
            provider_name: 'command',
            tool_calls_count: 0,
            tools_called: [],
            execution_time_ms: 2,
          },
        }),
      });
      return;
    }
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

  // The server's own catalogue: the palette renders whatever this returns.
  await page.route('**/api/commands**', async (route) => {
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({
        commands: [
          {
            name: 'coach-list',
            command: '/coach list',
            args: null,
            description: 'List the coaches you can add to a chat',
            domain: 'coach',
          },
          {
            name: 'group-create',
            command: '/group create',
            args: '<name>',
            description: 'Create a coaching group',
            domain: 'group',
          },
        ],
      }),
    });
  });

  // The athlete's coach list: one installed coach (a system coach with an assignment
  // row — the resolver admits those), one personal coach with no handle, and one
  // catalogue coach that is listed but never installed, so `@` must not offer it.
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
            is_assigned: true,
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
            is_assigned: true,
          },
          {
            id: 'coach-catalogue',
            title: 'Recovery Catalogue',
            description: 'Never installed',
            system_prompt: 'You are a catalogue coach.',
            category: 'recovery',
            tags: [],
            token_count: 100,
            is_favorite: false,
            use_count: 0,
            last_used_at: null,
            created_at: '2026-01-01T00:00:00Z',
            updated_at: '2026-01-01T00:00:00Z',
            is_system: true,
            is_assigned: false,
            handle: 'recovery-catalogue',
          },
        ],
        total: 3,
      }),
    });
  });

  return { created, sent };
}

test.describe('Chat-first shell', () => {
  test('a regular user lands on chat, and the sidebar offers Discover but no Coaches tab', async ({ page }) => {
    await setupShellMocks(page);
    await loginToDashboard(page);
    await page.waitForSelector('aside', { timeout: 10000 });

    await expect(page).toHaveURL(/#chat$/);
    await expect(page.getByTestId('chat-empty-state')).toBeVisible({ timeout: 10000 });

    const aside = page.locator('aside');
    await expect(aside.getByRole('button', { name: 'Chat' })).toBeVisible();
    await expect(aside.getByRole('button', { name: 'Discover', exact: true })).toBeVisible();
    await expect(aside.getByRole('button', { name: 'Groups' })).toHaveCount(0);
    await expect(aside.getByRole('button', { name: 'Coaches' })).toHaveCount(0);
  });

  test('a stale #groups deep link lands on chat', async ({ page }) => {
    await setupShellMocks(page);
    await loginToDashboard(page);
    await page.waitForSelector('aside', { timeout: 10000 });

    await page.goto('/#groups/group-1');
    await page.waitForSelector('aside', { timeout: 10000 });

    await expect(page).toHaveURL(/#chat$/);
    await expect(page.getByTestId('conversation-list')).toBeVisible();
  });

  test('the empty pane names what to do and offers the "+" and Commands', async ({ page }) => {
    await setupShellMocks(page);
    await loginToDashboard(page);
    await page.waitForSelector('aside', { timeout: 10000 });

    const empty = page.getByTestId('chat-empty-state');
    await expect(empty).toBeVisible({ timeout: 10000 });
    await expect(empty.getByText('Pick a chat, or start one')).toBeVisible();
    await expect(page.getByTestId('chat-empty-commands')).toBeVisible();
  });

  test('the list row carries its unread count, preview and time', async ({ page }) => {
    await setupShellMocks(page);
    await loginToDashboard(page);
    await page.waitForSelector('aside', { timeout: 10000 });

    const row = page.locator('[data-testid="conversation-row"]', { hasText: 'Sunday long run' });
    await expect(row.getByTestId('conversation-unread-count')).toHaveText('3', { timeout: 10000 });
    await expect(row.getByTestId('conversation-preview')).toHaveText('How did the long run feel?');
    await expect(row.getByTestId('conversation-timestamp')).not.toBeEmpty();
  });

  test('the "/" button beside the composer opens the server catalogue', async ({ page }) => {
    await setupShellMocks(page);
    await loginToDashboard(page);
    await page.waitForSelector('aside', { timeout: 10000 });

    await page.goto('/#chat/conv-1');
    await page.waitForSelector('aside', { timeout: 10000 });

    await page.getByTestId('slash-command-button').click();

    await expect(page.getByPlaceholder('Message Dravr...')).toHaveValue('/');
    const palette = page.getByTestId('command-palette');
    await expect(palette).toBeVisible();
    await expect(palette.getByText('/coach list')).toBeVisible();
    await expect(palette.getByText('/group create')).toBeVisible();
  });

  test('the thread header is a button that opens the info drawer', async ({ page }) => {
    await setupShellMocks(page);
    await loginToDashboard(page);
    await page.waitForSelector('aside', { timeout: 10000 });

    await page.goto('/#chat/conv-1');
    await page.waitForSelector('aside', { timeout: 10000 });

    const header = page.getByTestId('conversation-header-title');
    await expect(header).toHaveAttribute('aria-haspopup', 'dialog');
    await header.click();

    await expect(page.getByTestId('conversation-info-panel')).toBeVisible();
    await expect(page.getByRole('dialog', { name: 'Chat info' })).toBeVisible();
    await expect(page.getByTestId('plain-info-panel')).toBeVisible();
  });

  test('a stale #my-coaches deep link lands on chat', async ({ page }) => {
    await setupShellMocks(page);
    await loginToDashboard(page);
    await page.waitForSelector('aside', { timeout: 10000 });

    await page.goto('/#my-coaches');
    await page.waitForSelector('aside', { timeout: 10000 });

    await expect(page.getByTestId('chat-empty-state')).toBeVisible({ timeout: 10000 });
    await expect(page).toHaveURL(/#chat$/);
    await expect(page.getByText('custom AI personas')).toHaveCount(0);
  });

  test('the "+" offers a new chat and a new group chat, and starts a plain chat', async ({ page }) => {
    const { created } = await setupShellMocks(page);
    await loginToDashboard(page);
    await page.waitForSelector('aside', { timeout: 10000 });

    await page.getByRole('button', { name: 'New', exact: true }).first().click();
    const menu = page.getByRole('menu', { name: 'Start a conversation' });
    await expect(menu.getByRole('menuitem')).toHaveText(['New chat', 'New group chat']);

    await menu.getByRole('menuitem', { name: 'New chat' }).click();

    await expect.poll(() => created.length).toBe(1);
    expect(created[0].body).not.toHaveProperty('group_id');
    await expect(page).toHaveURL(/#chat\/conv-new$/);
  });

  test('"New group chat" asks for a name and sends /group create, creating no group itself', async ({ page }) => {
    const { created, sent } = await setupShellMocks(page);
    await loginToDashboard(page);
    await page.waitForSelector('aside', { timeout: 10000 });

    await page.goto('/#chat/conv-1');
    await page.waitForSelector('aside', { timeout: 10000 });

    await page.getByRole('button', { name: 'New', exact: true }).first().click();
    await page.getByRole('menuitem', { name: 'New group chat' }).click();

    await page.getByTestId('group-name-input').fill('Sunday Riders');
    await page.getByTestId('group-name-submit').click();

    await expect.poll(() => sent.length, { timeout: 10000 }).toBe(1);
    expect(sent[0]).toBe('/group create Sunday Riders');
    // Nothing POSTed a group, and no second conversation was created either.
    expect(created).toHaveLength(0);
  });

  test('with a conversation open, "+" also offers adding someone, which opens the info drawer on Participants', async ({ page }) => {
    await setupShellMocks(page);
    await loginToDashboard(page);
    await page.waitForSelector('aside', { timeout: 10000 });

    await page.goto('/#chat/conv-1');
    await page.waitForSelector('aside', { timeout: 10000 });
    await expect(page.getByTestId('conversation-header-title')).toHaveText('Sunday long run');

    await page.getByRole('button', { name: 'New', exact: true }).first().click();
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
    // Only the installed coach is offered: the handle-less personal coach cannot be
    // mentioned, and the catalogue coach has no assignment row so it would not route.
    await expect(palette.getByRole('option')).toHaveCount(1);
    await expect(palette.getByText('@recovery-coach')).toBeVisible();
    await expect(palette.getByText('My Custom Coach')).toHaveCount(0);
    await expect(palette.getByText('@recovery-catalogue')).toHaveCount(0);

    await page.getByTestId('mention-palette-option-recovery-coach').click();

    await expect(composer).toHaveValue('Hey @recovery-coach ');
    await expect(palette).toHaveCount(0);
  });
});
