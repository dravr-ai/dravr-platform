// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Playwright E2E tests for the Activity tab (previously Monitor).
// ABOUTME: Tests real-time activity feed, summary stats, LLM calls list, and empty states.

import { test, expect, type Page } from '@playwright/test';
import { setupDashboardMocks, loginToDashboard, navigateToTab } from './test-helpers';

// Helper to set up authenticated state with Activity API mocks
async function setupActivityMocks(
  page: Page,
  options: {
    hasData?: boolean;
  } = {}
) {
  const { hasData = true } = options;

  // Set up base dashboard mocks (includes login mock)
  await setupDashboardMocks(page, { role: 'admin' });

  // Mock recent activity endpoint
  await page.route('**/api/admin/analytics/recent-activity', async (route) => {
    if (!hasData) {
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({
          recent_llm_calls: [],
          recent_conversations: [],
          summary: {
            active_conversations: 0,
            llm_calls_today: 0,
            total_tokens_today: 0,
            estimated_cost_today: 0,
          },
        }),
      });
      return;
    }

    const llmCalls = [
      { id: 'llm-1', provider: 'google', model: 'gemini-2.0-flash', total_tokens: 1500, cost_usd: 0.0012, call_type: 'chat', execution_time_ms: 230, created_at: new Date(Date.now() - 60000).toISOString() },
      { id: 'llm-2', provider: 'groq', model: 'llama-3.3-70b', total_tokens: 800, cost_usd: 0.0005, call_type: 'insight', execution_time_ms: 150, created_at: new Date(Date.now() - 120000).toISOString() },
      { id: 'llm-3', provider: 'google', model: 'gemini-2.0-flash', total_tokens: 2200, cost_usd: 0.0018, call_type: 'chat', execution_time_ms: 340, created_at: new Date(Date.now() - 300000).toISOString() },
    ];

    const conversations = [
      { id: 'conv-1', title: 'Training Plan Review', updated_at: new Date(Date.now() - 30000).toISOString(), user_email: 'alice@acme.com' },
      { id: 'conv-2', title: 'Recovery Analysis', updated_at: new Date(Date.now() - 180000).toISOString(), user_email: 'bob@acme.com' },
    ];

    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({
        recent_llm_calls: llmCalls,
        recent_conversations: conversations,
        summary: {
          active_conversations: 3,
          llm_calls_today: 42,
          total_tokens_today: 58000,
          estimated_cost_today: 0.045,
        },
      }),
    });
  });
}

async function loginAndNavigateToActivity(page: Page) {
  await loginToDashboard(page);
  await navigateToTab(page, 'Activity');
  await page.waitForTimeout(500);
}

test.describe('Activity Tab - Summary Stats', () => {
  test('renders Activity tab with header', async ({ page }) => {
    await setupActivityMocks(page);
    await loginAndNavigateToActivity(page);

    // Check header — Dashboard top bar shows the active tab name
    await expect(page.getByRole('heading', { name: 'Activity', level: 1 })).toBeVisible();
  });

  test('displays Active Conversations stat card', async ({ page }) => {
    await setupActivityMocks(page);
    await loginAndNavigateToActivity(page);

    await expect(page.getByText('Active Conversations (15m)')).toBeVisible();
    await expect(page.locator('.stat-card-dark').filter({ hasText: 'Active Conversations' }).getByText('3')).toBeVisible();
  });

  test('displays LLM Calls Today stat card', async ({ page }) => {
    await setupActivityMocks(page);
    await loginAndNavigateToActivity(page);

    await expect(page.getByText('LLM Calls Today')).toBeVisible();
    await expect(page.locator('.stat-card-dark').filter({ hasText: 'LLM Calls Today' }).getByText('42')).toBeVisible();
  });

  test('displays Tokens Today stat card', async ({ page }) => {
    await setupActivityMocks(page);
    await loginAndNavigateToActivity(page);

    await expect(page.getByText('Tokens Today')).toBeVisible();
  });

  test('displays Est. Cost Today stat card', async ({ page }) => {
    await setupActivityMocks(page);
    await loginAndNavigateToActivity(page);

    await expect(page.getByText('Est. Cost Today')).toBeVisible();
  });
});

test.describe('Activity Tab - LLM Calls List', () => {
  test('displays Recent LLM Calls section', async ({ page }) => {
    await setupActivityMocks(page);
    await loginAndNavigateToActivity(page);

    await expect(page.getByText('Recent LLM Calls')).toBeVisible();
  });

  test('displays LLM call entries with provider/model', async ({ page }) => {
    await setupActivityMocks(page);
    await loginAndNavigateToActivity(page);

    await expect(page.getByText('google/gemini-2.0-flash').first()).toBeVisible();
    await expect(page.getByText('groq/llama-3.3-70b')).toBeVisible();
  });

  test('displays call type badges', async ({ page }) => {
    await setupActivityMocks(page);
    await loginAndNavigateToActivity(page);

    await expect(page.getByText('chat').first()).toBeVisible();
    await expect(page.getByText('insight')).toBeVisible();
  });

  test('displays token counts', async ({ page }) => {
    await setupActivityMocks(page);
    await loginAndNavigateToActivity(page);

    // Token counts are formatted (e.g. "1.5K tokens")
    await expect(page.getByText('1.5K tokens')).toBeVisible();
  });
});

test.describe('Activity Tab - Conversations List', () => {
  test('displays Recent Conversations section', async ({ page }) => {
    await setupActivityMocks(page);
    await loginAndNavigateToActivity(page);

    await expect(page.getByText('Recent Conversations')).toBeVisible();
  });

  test('displays conversation titles', async ({ page }) => {
    await setupActivityMocks(page);
    await loginAndNavigateToActivity(page);

    await expect(page.getByText('Training Plan Review')).toBeVisible();
    await expect(page.getByText('Recovery Analysis')).toBeVisible();
  });

  test('displays user emails', async ({ page }) => {
    await setupActivityMocks(page);
    await loginAndNavigateToActivity(page);

    await expect(page.getByText('alice@acme.com')).toBeVisible();
    await expect(page.getByText('bob@acme.com')).toBeVisible();
  });
});

test.describe('Activity Tab - Empty State', () => {
  test('shows empty state message when no activity', async ({ page }) => {
    await setupActivityMocks(page, { hasData: false });
    await loginAndNavigateToActivity(page);

    await expect(page.getByText('No activity yet')).toBeVisible();
    await expect(page.getByText('LLM calls and conversations will appear here in real-time.')).toBeVisible();
  });

  test('shows zero stats when no activity', async ({ page }) => {
    await setupActivityMocks(page, { hasData: false });
    await loginAndNavigateToActivity(page);

    // All stat cards should show 0
    const statCards = page.locator('.stat-card-dark');
    await expect(statCards.filter({ hasText: 'LLM Calls Today' }).getByText('0')).toBeVisible();
  });
});

test.describe('Activity Tab - Loading State', () => {
  test('shows loading spinner while data loads', async ({ page }) => {
    await setupDashboardMocks(page, { role: 'admin' });

    await page.route('**/api/admin/analytics/recent-activity', async (route) => {
      await new Promise((resolve) => setTimeout(resolve, 1000));
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({
          recent_llm_calls: [],
          recent_conversations: [],
          summary: { active_conversations: 0, llm_calls_today: 0, total_tokens_today: 0, estimated_cost_today: 0 },
        }),
      });
    });

    await loginToDashboard(page);
    await navigateToTab(page, 'Activity');

    // Should show loading spinner
    await expect(page.locator('.pierre-spinner')).toBeVisible({ timeout: 5000 });
  });
});
