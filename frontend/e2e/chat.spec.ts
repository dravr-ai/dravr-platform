// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Playwright E2E tests for Chat and Messaging features.
// ABOUTME: Tests conversation list, message display, prompt suggestions, and CRUD operations.

import { test, expect, type Page } from '@playwright/test';
import { setupDashboardMocks, loginToDashboard } from './test-helpers';

// Mock conversations matching ConversationsResponse
const mockConversations = {
  conversations: [
    {
      id: 'conv-1',
      title: 'Marathon Training Plan',
      coach_id: 'coach-marathon',
      coach_name: 'Marathon Coach',
      created_at: '2024-06-01T10:00:00Z',
      updated_at: '2024-06-01T12:00:00Z',
      message_count: 5,
    },
    {
      id: 'conv-2',
      title: 'Nutrition Strategy',
      coach_id: 'coach-nutrition',
      coach_name: 'Nutrition Coach',
      created_at: '2024-05-28T08:00:00Z',
      updated_at: '2024-05-30T09:00:00Z',
      message_count: 3,
    },
    {
      id: 'conv-3',
      title: 'Recovery Protocol',
      coach_id: 'coach-recovery',
      coach_name: 'Recovery Coach',
      created_at: '2024-05-20T14:00:00Z',
      updated_at: '2024-05-25T16:00:00Z',
      message_count: 8,
    },
  ],
  total: 3,
  limit: 50,
  offset: 0,
};

// Mock messages for a conversation
const mockMessages = {
  messages: [
    {
      id: 'msg-1',
      conversation_id: 'conv-1',
      role: 'user',
      content: 'What should my weekly mileage be for a marathon?',
      created_at: '2024-06-01T10:00:00Z',
    },
    {
      id: 'msg-2',
      conversation_id: 'conv-1',
      role: 'assistant',
      content: 'For marathon training, your weekly mileage should gradually build up. Based on your current fitness level, I recommend starting at **30-35 miles per week** and building to 40-50 miles over 12-16 weeks.\n\n### Key principles:\n1. Increase mileage by no more than 10% per week\n2. Include one long run per week\n3. Add recovery weeks every 3-4 weeks',
      created_at: '2024-06-01T10:01:00Z',
      model: 'gemini-1.5-flash',
      execution_time_ms: 1500,
    },
    {
      id: 'msg-3',
      conversation_id: 'conv-1',
      role: 'user',
      content: 'How should I taper before race day?',
      created_at: '2024-06-01T10:05:00Z',
    },
    {
      id: 'msg-4',
      conversation_id: 'conv-1',
      role: 'assistant',
      content: 'Start your taper 2-3 weeks before race day. Reduce volume by 20-30% each week while maintaining intensity.',
      created_at: '2024-06-01T10:06:00Z',
      model: 'gemini-1.5-flash',
      execution_time_ms: 1200,
    },
  ],
};

async function setupChatMocks(page: Page, options: { emptyConversations?: boolean } = {}) {
  const { emptyConversations = false } = options;

  // Set up base dashboard mocks FIRST; specific mocks registered AFTER take
  // priority because Playwright checks routes in LIFO (last registered first).
  await setupDashboardMocks(page, { role: 'user' });

  // Conversation messages endpoints (must be before conversations catch-all)
  await page.route('**/api/chat/conversations/*/messages', async (route, request) => {
    if (request.method() === 'GET') {
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify(mockMessages),
      });
    } else if (request.method() === 'POST') {
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({
          turn_id: '00000000-0000-4000-8000-000000000001',
          user_message: {
            id: 'msg-new-user',
            conversation_id: 'conv-1',
            role: 'user',
            content: 'Test message',
            created_at: new Date().toISOString(),
          },
          assistant: {
            message: {
              id: 'msg-new-assistant',
              conversation_id: 'conv-1',
              role: 'assistant',
              content: 'Here is the assistant response to your message.',
              created_at: new Date().toISOString(),
            },
            // The server decided what this surface renders; the client lays
            // out the blocks it is given.
            blocks: [
              { type: 'prose', text: 'Here is the assistant response to your message.' },
            ],
            finish_reason: 'stop',
          },
          conversation_updated_at: new Date().toISOString(),
          telemetry: {
            model: 'gemini-1.5-flash',
            provider_name: 'gemini',
            tool_calls_count: 0,
            tools_called: [],
            execution_time_ms: 1000,
          },
        }),
      });
    } else {
      await route.fallback();
    }
  });

  // Individual conversation CRUD (update/delete) - must match before conversations list
  await page.route(/\/api\/chat\/conversations\/[^/]+$/, async (route, request) => {
    if (request.method() === 'DELETE') {
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({ success: true }),
      });
    } else if (request.method() === 'PATCH' || request.method() === 'PUT') {
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({
          ...mockConversations.conversations[0],
          title: 'Renamed Conversation',
        }),
      });
    } else {
      await route.fallback();
    }
  });

  // Conversations list - use regex to match query params but NOT sub-paths like /messages
  await page.route(/\/api\/chat\/conversations(\?.*)?$/, async (route, request) => {
    if (request.method() === 'GET') {
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify(
          emptyConversations
            ? { conversations: [], total: 0, limit: 50, offset: 0 }
            : mockConversations
        ),
      });
    } else if (request.method() === 'POST') {
      await route.fulfill({
        status: 201,
        contentType: 'application/json',
        body: JSON.stringify({
          id: 'conv-new',
          title: 'New Conversation',
          coach_id: null,
          coach_name: null,
          created_at: new Date().toISOString(),
          updated_at: new Date().toISOString(),
          message_count: 0,
        }),
      });
    } else {
      await route.fallback();
    }
  });

  // Providers status (needed by ChatTab)
  await page.route('**/api/providers', async (route) => {
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({
        providers: [
          { provider: 'strava', display_name: 'Strava', requires_oauth: true, connected: true, capabilities: ['activities', 'athlete'] },
        ],
      }),
    });
  });

  // Coaches (the chat header and the @handle palette read this list)
  await page.route('**/api/coaches**', async (route) => {
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({
        coaches: [
          {
            id: 'coach-training',
            title: 'Training Coach',
            description: 'Personalized training plans and workout analysis',
            system_prompt: 'You are a training coach.',
            category: 'training',
            is_system: true,
            is_favorite: false,
            use_count: 10,
            created_at: '2024-01-01T00:00:00Z',
            updated_at: '2024-01-01T00:00:00Z',
          },
          {
            id: 'coach-nutrition',
            title: 'Nutrition Coach',
            description: 'Dietary guidance and meal planning',
            system_prompt: 'You are a nutrition coach.',
            category: 'nutrition',
            is_system: true,
            is_favorite: false,
            use_count: 5,
            created_at: '2024-01-01T00:00:00Z',
            updated_at: '2024-01-01T00:00:00Z',
          },
          {
            id: 'coach-recovery',
            title: 'Recovery Coach',
            description: 'Sleep and recovery optimization',
            system_prompt: 'You are a recovery coach.',
            category: 'recovery',
            is_system: true,
            is_favorite: false,
            use_count: 3,
            created_at: '2024-01-01T00:00:00Z',
            updated_at: '2024-01-01T00:00:00Z',
          },
          {
            id: 'coach-recipes',
            title: 'Recipe Coach',
            description: 'Healthy athlete recipes and meal prep',
            system_prompt: 'You are a recipe coach.',
            category: 'recipes',
            is_system: true,
            is_favorite: false,
            use_count: 2,
            created_at: '2024-01-01T00:00:00Z',
            updated_at: '2024-01-01T00:00:00Z',
          },
        ],
        total: 4,
        metadata: { timestamp: '2024-06-01T10:00:00Z', api_version: 'v1' },
      }),
    });
  });

}

test.describe('Chat - Empty pane', () => {
  test.beforeEach(async ({ page }) => {
    await setupChatMocks(page, { emptyConversations: true });
    await loginToDashboard(page);
  });

  test('shows one line, the "+" and the Commands button', async ({ page }) => {
    const empty = page.getByTestId('chat-empty-state');
    await expect(empty).toBeVisible({ timeout: 10000 });
    await expect(empty.getByText('Pick a chat, or start one')).toBeVisible();
    await expect(empty.getByRole('button', { name: 'New', exact: true })).toBeVisible();
    await expect(page.getByTestId('chat-empty-commands')).toBeVisible();
  });

  test('offers no coach grid and no coach creation', async ({ page }) => {
    await expect(page.getByTestId('chat-empty-state')).toBeVisible({ timeout: 10000 });
    await expect(page.locator('h4', { hasText: 'System Coaches' })).toHaveCount(0);
    await expect(page.getByRole('button', { name: /Create Coach/i })).toHaveCount(0);
    await expect(page.getByText('Training Coach')).toHaveCount(0);
  });
});

test.describe('Chat - Conversation Sidebar', () => {
  test.beforeEach(async ({ page }) => {
    await setupChatMocks(page);
    await loginToDashboard(page);
  });

  test('pins a search box above one flat list of every conversation', async ({ page }) => {
    await expect(page.getByLabel('Search conversations')).toBeVisible({ timeout: 10000 });
    await expect(page.getByRole('list', { name: 'Conversations' })).toBeVisible();
  });

  test('displays existing conversations in sidebar', async ({ page }) => {
    // Conversations appear as flat rows in the sidebar list
    await expect(page.getByText('Marathon Training Plan')).toBeVisible({ timeout: 10000 });
    await expect(page.getByText('Nutrition Strategy')).toBeVisible();
    await expect(page.getByText('Recovery Protocol')).toBeVisible();
  });

  test('shows the row actions menu on conversation hover', async ({ page }) => {
    await expect(page.getByText('Marathon Training Plan')).toBeVisible({ timeout: 10000 });

    // Hover over conversation item (button with conversation title text)
    const conversationItem = page.locator('[data-testid="conversation-row"]', {
      hasText: 'Marathon Training Plan',
    });
    await conversationItem.hover();

    // The three actions live behind one trigger per row, so the row itself
    // stays clickable while the pointer is on it.
    await conversationItem.getByTestId('conversation-actions-trigger').click();
    const menu = conversationItem.getByRole('menu', { name: 'Conversation actions' });
    await expect(menu.getByRole('menuitem', { name: 'Rename conversation' })).toBeVisible();
    await expect(menu.getByRole('menuitem', { name: 'Delete conversation' })).toBeVisible();
  });

  test('the menu rename enables edit mode with the current title', async ({ page }) => {
    await expect(page.getByText('Marathon Training Plan')).toBeVisible({ timeout: 10000 });

    const conversationItem = page.locator('[data-testid="conversation-row"]', {
      hasText: 'Marathon Training Plan',
    });
    await conversationItem.hover();
    await conversationItem.getByTestId('conversation-actions-trigger').click();
    await conversationItem.getByRole('menuitem', { name: 'Rename conversation' }).click();

    // Input field should appear with current title
    const input = page.getByLabel('Conversation title');
    await expect(input).toBeVisible({ timeout: 5000 });
    await expect(input).toHaveValue('Marathon Training Plan');
  });

  test('the menu delete opens a confirmation dialog', async ({ page }) => {
    await expect(page.getByText('Marathon Training Plan')).toBeVisible({ timeout: 10000 });

    const conversationItem = page.locator('[data-testid="conversation-row"]', {
      hasText: 'Marathon Training Plan',
    });
    await conversationItem.hover();
    await conversationItem.getByTestId('conversation-actions-trigger').click();
    await conversationItem.getByRole('menuitem', { name: 'Delete conversation' }).click();

    // The list's ConfirmDialog shows a "Delete Conversation" title
    const dialog = page.locator('[role="dialog"], .fixed.inset-0').last();
    await expect(page.getByText('Delete Conversation')).toBeVisible({ timeout: 5000 });
    await expect(dialog.getByRole('button', { name: 'Delete' })).toBeVisible();
    await expect(dialog.getByRole('button', { name: 'Cancel' })).toBeVisible();
  });

  test('confirming delete calls the API', async ({ page }) => {
    let deleteCalled = false;
    await page.route(/\/api\/chat\/conversations\/conv-1$/, async (route, request) => {
      if (request.method() === 'DELETE') {
        deleteCalled = true;
        await route.fulfill({
          status: 200,
          contentType: 'application/json',
          body: JSON.stringify({ success: true }),
        });
      } else {
        await route.fallback();
      }
    });

    await expect(page.getByText('Marathon Training Plan')).toBeVisible({ timeout: 10000 });

    const conversationItem = page.locator('[data-testid="conversation-row"]', {
      hasText: 'Marathon Training Plan',
    });
    await conversationItem.hover();
    await conversationItem.getByTestId('conversation-actions-trigger').click();
    await conversationItem.getByRole('menuitem', { name: 'Delete conversation' }).click();

    // Click Delete in confirmation dialog
    const confirmDialog = page.locator('[role="dialog"], .fixed.inset-0').last();
    await expect(page.getByText('Delete Conversation')).toBeVisible({ timeout: 5000 });
    await confirmDialog.getByRole('button', { name: 'Delete' }).click();
    await page.waitForTimeout(500);

    expect(deleteCalled).toBe(true);
  });
});

test.describe('Chat - Messages Display', () => {
  test.beforeEach(async ({ page }) => {
    await setupChatMocks(page);
    await loginToDashboard(page);
  });

  test('clicking conversation loads messages', async ({ page }) => {
    await expect(page.getByText('Marathon Training Plan')).toBeVisible({ timeout: 10000 });

    // Click on conversation in sidebar
    await page.getByText('Marathon Training Plan').click();
    await page.waitForTimeout(500);

    // Messages should load in main content area
    await expect(page.getByText('What should my weekly mileage be for a marathon?')).toBeVisible({ timeout: 10000 });
    await expect(page.getByText(/30-35 miles per week/)).toBeVisible();
  });

  test('messages display with markdown formatting', async ({ page }) => {
    await expect(page.getByText('Marathon Training Plan')).toBeVisible({ timeout: 10000 });
    await page.getByText('Marathon Training Plan').click();
    await page.waitForTimeout(500);

    // Markdown heading should render
    await expect(page.getByText('Key principles:')).toBeVisible({ timeout: 10000 });
  });

  test('both user and assistant messages are displayed', async ({ page }) => {
    await expect(page.getByText('Marathon Training Plan')).toBeVisible({ timeout: 10000 });
    await page.getByText('Marathon Training Plan').click();
    await page.waitForTimeout(500);

    // User messages
    await expect(page.getByText('What should my weekly mileage be for a marathon?')).toBeVisible({ timeout: 10000 });
    await expect(page.getByText('How should I taper before race day?')).toBeVisible();
    // Assistant response
    await expect(page.getByText(/Start your taper 2-3 weeks/)).toBeVisible();
  });
});

test.describe('Chat - Empty State', () => {
  test('shows welcome state when no conversations exist', async ({ page }) => {
    await setupChatMocks(page, { emptyConversations: true });
    await loginToDashboard(page);

    // The empty pane names what to do next
    await expect(page.getByTestId('chat-empty-state')).toBeVisible({ timeout: 10000 });
    // No conversations in the sidebar list either
    await expect(page.getByTestId('conversation-list-empty')).toContainText('No chats yet');
  });
});

test.describe('Chat - Error Handling', () => {
  test('handles conversation load failure gracefully', async ({ page }) => {
    // Set up base dashboard mocks FIRST so specific overrides registered
    // AFTER take priority (Playwright uses LIFO route matching).
    await setupDashboardMocks(page, { role: 'user' });

    // Override conversations with error AFTER setupDashboardMocks
    await page.route('**/api/chat/conversations**', async (route) => {
      await route.fulfill({
        status: 500,
        contentType: 'application/json',
        body: JSON.stringify({ error: 'Internal server error' }),
      });
    });

    await page.route('**/api/providers', async (route) => {
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({ providers: [] }),
      });
    });

    await page.route('**/api/coaches**', async (route) => {
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({ coaches: [], total: 0 }),
      });
    });

    await loginToDashboard(page);

    // Page should still render without crashing
    await page.waitForTimeout(1000);
    await expect(page.locator('main')).toBeVisible();
  });
});

test.describe('Chat - Provider Connection', () => {
  test('shows no-provider description when no provider connected', async ({ page }) => {
    await setupChatMocks(page, { emptyConversations: true });

    // Override providers AFTER setupChatMocks so this takes priority (LIFO)
    await page.route('**/api/providers', async (route) => {
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({ providers: [] }),
      });
    });

    await loginToDashboard(page);

    // ChatTab shows "No provider connected" when no provider is connected
    await expect(page.getByText('No provider connected')).toBeVisible({ timeout: 10000 });
  });
});

test.describe('Chat - No provider connected', () => {
  test('shows connect-provider banner with deep link instead of a raw 403 error', async ({ page }) => {
    await setupChatMocks(page);

    // Providerless users are gated server-side: the messaging POST returns a
    // structured 403 NoProviderConnected. The chat must render the friendly
    // connect-provider banner (deep-linking to Data Providers), not "HTTP error".
    // Providerless: the status endpoint must report no connected provider so the
    // banner renders (registered after setupChatMocks so LIFO matching wins).
    await page.route('**/api/providers', async (route) => {
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({ providers: [] }),
      });
    });

    await page.route('**/api/chat/conversations/*/messages', async (route, request) => {
      if (request.method() === 'POST') {
        await route.fulfill({
          status: 403,
          contentType: 'application/json',
          body: JSON.stringify({
            code: 'NoProviderConnected',
            message: 'Connect a fitness provider before using messaging features',
            details: { action: 'connect_provider' },
          }),
        });
      } else {
        await route.fulfill({
          status: 200,
          contentType: 'application/json',
          body: JSON.stringify(mockMessages),
        });
      }
    });

    await loginToDashboard(page);
    // The composer belongs to an open thread — the chat pane with none open
    // shows the empty state, not a message box.
    await page.goto('/#chat/conv-1');
    await page.waitForSelector('aside', { timeout: 10000 });

    const input = page.getByPlaceholder('Message Dravr...');
    await expect(input).toBeVisible({ timeout: 10000 });
    await input.click();
    // pressSequentially types real keystrokes so React's controlled-input
    // onChange fires reliably (a plain fill() can be dropped on re-render).
    await input.pressSequentially("Quelle température fera-t-il à Prévost aujourd'hui?", { delay: 10 });
    const sendBtn = page.getByRole('button', { name: 'Send message' });
    await expect(sendBtn).toBeEnabled({ timeout: 5000 });
    await sendBtn.click();

    // Friendly nudge appears; raw error banner does not. Both the banner and
    // the 403's own copy say "Connect a fitness provider", so the nudge is
    // addressed by its testid rather than by a string the error strip shares.
    const banner = page.getByTestId('connect-provider-banner');
    await expect(banner.getByText('Connect a fitness provider')).toBeVisible({ timeout: 10000 });
    await expect(banner.getByRole('button', { name: 'Connect', exact: true })).toBeVisible();
    await expect(page.getByText(/HTTP error/i)).toHaveCount(0);
  });
});
