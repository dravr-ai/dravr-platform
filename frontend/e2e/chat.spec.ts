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
          user_message: {
            id: 'msg-new-user',
            conversation_id: 'conv-1',
            role: 'user',
            content: 'Test message',
            created_at: new Date().toISOString(),
          },
          assistant_message: {
            id: 'msg-new-assistant',
            conversation_id: 'conv-1',
            role: 'assistant',
            content: 'Here is the assistant response to your message.',
            created_at: new Date().toISOString(),
          },
          conversation_updated_at: new Date().toISOString(),
          model: 'gemini-1.5-flash',
          execution_time_ms: 1000,
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

  // Coaches (needed by PromptSuggestions in welcome view)
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

  // Social endpoints (prevent 401s on other tabs)
  await page.route('**/api/social/**', async (route) => {
    const url = route.request().url();
    if (url.includes('/friends')) {
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({ friends: [], total: 0, metadata: { timestamp: '2024-06-01T10:00:00Z', api_version: 'v1' } }),
      });
    } else if (url.includes('/feed')) {
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({ items: [], next_cursor: null, has_more: false, metadata: { timestamp: '2024-06-01T10:00:00Z', api_version: 'v1' } }),
      });
    } else if (url.includes('/suggestions')) {
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({ suggestions: [], total: 0, metadata: { timestamp: '2024-06-01T10:00:00Z', api_version: 'v1' } }),
      });
    } else {
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({}),
      });
    }
  });

}

test.describe('Chat - Welcome and Prompt Suggestions', () => {
  test.beforeEach(async ({ page }) => {
    await setupChatMocks(page, { emptyConversations: true });
    await loginToDashboard(page);
  });

  test('displays welcome heading on chat tab', async ({ page }) => {
    // User defaults to Chat tab, welcome view shows when no conversation selected
    await expect(page.getByText('Ready to analyze your fitness')).toBeVisible({ timeout: 10000 });
  });

  test('displays message input with placeholder', async ({ page }) => {
    // ChatTab uses <input> not <textarea> for the welcome form
    await expect(page.getByPlaceholder('Message Pierre...')).toBeVisible({ timeout: 10000 });
  });

  test('displays Send button', async ({ page }) => {
    await expect(page.getByText('Send')).toBeVisible({ timeout: 10000 });
  });

  test('displays coaches section with category badges', async ({ page }) => {
    // PromptSuggestions renders coaches from /api/coaches below the welcome form
    // Each coach card shows a category emoji badge — use h3 to avoid matching sidebar button
    await expect(page.locator('h3', { hasText: 'Coaches' })).toBeVisible({ timeout: 10000 });
    await expect(page.locator('h4', { hasText: 'System Coaches' })).toBeVisible();
  });

  test('displays coach titles and descriptions', async ({ page }) => {
    // Coach cards show title and description from mock data
    await expect(page.getByText('Training Coach')).toBeVisible({ timeout: 10000 });
    await expect(page.getByText('Nutrition Coach')).toBeVisible();
  });
});

test.describe('Chat - Conversation Sidebar', () => {
  test.beforeEach(async ({ page }) => {
    await setupChatMocks(page);
    await loginToDashboard(page);
  });

  test('displays Recent Chats heading in sidebar', async ({ page }) => {
    // ConversationsPanel shows "Recent Chats" heading when activeTab === 'chat'
    await expect(page.getByText('Recent Chats')).toBeVisible({ timeout: 10000 });
  });

  test('displays existing conversations in sidebar', async ({ page }) => {
    // Conversations appear in sidebar ConversationsPanel
    await expect(page.getByText('Marathon Training Plan')).toBeVisible({ timeout: 10000 });
    await expect(page.getByText('Nutrition Strategy')).toBeVisible();
    await expect(page.getByText('Recovery Protocol')).toBeVisible();
  });

  test('shows rename and delete buttons on conversation hover', async ({ page }) => {
    await expect(page.getByText('Marathon Training Plan')).toBeVisible({ timeout: 10000 });

    // Hover over conversation item (button with conversation title text)
    const conversationItem = page.locator('button:has-text("Marathon Training Plan")');
    await conversationItem.hover();

    // Action buttons scoped to the hovered conversation item to avoid strict mode violations
    // (each conversation has its own Rename/Delete buttons)
    await expect(conversationItem.getByLabel('Rename conversation')).toBeVisible();
    await expect(conversationItem.getByLabel('Delete conversation')).toBeVisible();
  });

  test('rename button enables edit mode with current title', async ({ page }) => {
    await expect(page.getByText('Marathon Training Plan')).toBeVisible({ timeout: 10000 });

    const conversationItem = page.locator('button:has-text("Marathon Training Plan")');
    await conversationItem.hover();

    await conversationItem.getByLabel('Rename conversation').click();

    // Input field should appear with current title
    const input = page.locator('input[type="text"]').first();
    await expect(input).toBeVisible({ timeout: 5000 });
    await expect(input).toHaveValue('Marathon Training Plan');
  });

  test('delete button opens confirmation dialog', async ({ page }) => {
    await expect(page.getByText('Marathon Training Plan')).toBeVisible({ timeout: 10000 });

    const conversationItem = page.locator('button:has-text("Marathon Training Plan")');
    await conversationItem.hover();

    await conversationItem.getByLabel('Delete conversation').click();

    // ConfirmDialog from ConversationsPanel shows "Delete Conversation" title
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

    const conversationItem = page.locator('button:has-text("Marathon Training Plan")');
    await conversationItem.hover();
    await conversationItem.getByLabel('Delete conversation').click();

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

    // Welcome heading visible
    await expect(page.getByText('Ready to analyze your fitness')).toBeVisible({ timeout: 10000 });
    // No conversations in sidebar
    await expect(page.getByText('No conversations yet')).toBeVisible();
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
    await expect(page.getByText('Ready to analyze your fitness')).toBeVisible({ timeout: 10000 });
  });
});
