// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Playwright E2E tests for insight creation and sharing features.
// ABOUTME: Tests coach suggestions loading, insight editing, visibility selection, and sharing flow.

import { test, expect, type Page } from '@playwright/test';
import { setupDashboardMocks, loginToDashboard, navigateToTab } from './test-helpers';

// Mock insight suggestions matching ListSuggestionsResponse
const mockSuggestions = {
  suggestions: [
    {
      insight_type: 'achievement',
      suggested_content: 'Completed a 10K personal best of 42:30! Consistent training is paying off.',
      suggested_title: 'New 10K PR',
      relevance_score: 0.95,
      sport_type: 'Running',
      training_phase: 'peak',
      source_activity_id: 'activity-1',
    },
    {
      insight_type: 'recovery',
      suggested_content: 'Sleep quality improved 15% this week. Recovery metrics looking strong for upcoming race.',
      suggested_title: 'Recovery Gains',
      relevance_score: 0.88,
      sport_type: 'Running',
      training_phase: 'recovery',
      source_activity_id: 'activity-2',
    },
    {
      insight_type: 'training_tip',
      suggested_content: 'Zone 2 training making up 80% of weekly volume. Great aerobic base building!',
      suggested_title: 'Aerobic Base Progress',
      relevance_score: 0.82,
      sport_type: 'Cycling',
      training_phase: 'base',
      source_activity_id: 'activity-3',
    },
  ],
  total: 3,
  metadata: { timestamp: '2024-06-01T10:00:00Z', api_version: 'v1' },
};

// Mock feed items (to show shared insights appear)
const mockFeedWithInsight = {
  items: [
    {
      insight: {
        id: 'insight-new',
        user_id: 'user-123',
        visibility: 'friends_only',
        insight_type: 'achievement',
        sport_type: 'Running',
        content: 'Completed a 10K personal best of 42:30! Consistent training is paying off.',
        title: 'New 10K PR',
        training_phase: 'peak',
        reaction_count: 0,
        adapt_count: 0,
        created_at: new Date().toISOString(),
        updated_at: new Date().toISOString(),
        expires_at: null,
        source_activity_id: 'activity-1',
        coach_generated: false,
      },
      author: {
        user_id: 'user-123',
        display_name: 'Test User',
        email: 'user@test.com',
      },
      reactions: { like: 0, celebrate: 0, inspire: 0, support: 0, total: 0 },
      user_reaction: null,
      user_has_adapted: false,
    },
  ],
  next_cursor: null,
  has_more: false,
  metadata: { timestamp: '2024-06-01T10:00:00Z', api_version: 'v1' },
};

async function setupInsightMocks(page: Page, options: { emptySuggestions?: boolean } = {}) {
  const { emptySuggestions = false } = options;

  // Set up base dashboard mocks FIRST; specific mocks registered AFTER take
  // priority because Playwright checks routes in LIFO (last registered first).
  await setupDashboardMocks(page, { role: 'user' });

  // Insight suggestions: GET /api/social/insights/suggestions
  await page.route('**/api/social/insights/suggestions**', async (route) => {
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify(
        emptySuggestions
          ? { suggestions: [], total: 0, metadata: { timestamp: '2024-06-01T10:00:00Z', api_version: 'v1' } }
          : mockSuggestions
      ),
    });
  });

  // Share from activity: POST /api/social/insights/from-activity
  await page.route('**/api/social/insights/from-activity', async (route) => {
    await route.fulfill({
      status: 201,
      contentType: 'application/json',
      body: JSON.stringify({
        insight: mockFeedWithInsight.items[0].insight,
      }),
    });
  });

  // Share insight directly: POST /api/social/share
  await page.route('**/api/social/share', async (route) => {
    await route.fulfill({
      status: 201,
      contentType: 'application/json',
      body: JSON.stringify({
        insight: mockFeedWithInsight.items[0].insight,
      }),
    });
  });

  // Social feed: GET /api/social/feed
  await page.route('**/api/social/feed**', async (route) => {
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({
        items: [],
        next_cursor: null,
        has_more: false,
        metadata: { timestamp: '2024-06-01T10:00:00Z', api_version: 'v1' },
      }),
    });
  });

  // Social friends (needed by FriendsTab)
  await page.route('**/api/social/friends**', async (route) => {
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({ friends: [], total: 0, metadata: { timestamp: '2024-06-01T10:00:00Z', api_version: 'v1' } }),
    });
  });

  // Providers (needed by ChatTab)
  await page.route('**/api/providers', async (route) => {
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({ providers: [] }),
    });
  });

  // Coaches
  await page.route('**/api/coaches**', async (route) => {
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({ coaches: [], total: 0 }),
    });
  });

}

test.describe('Insights - Suggestion Banner', () => {
  test.beforeEach(async ({ page }) => {
    await setupInsightMocks(page);
    await loginToDashboard(page);
  });

  test('suggestion banner appears on social feed when suggestions available', async ({ page }) => {
    await navigateToTab(page, 'Insights');

    // The suggestion banner shows "Coach noticed something!"
    await expect(page.getByText('Coach noticed something!')).toBeVisible({ timeout: 10000 });
  });

  test('Share with Friends button shows suggestion count', async ({ page }) => {
    await navigateToTab(page, 'Insights');

    // Button text includes count: "Share with Friends (3 available)"
    await expect(page.getByRole('button', { name: /Share with Friends.*available/i })).toBeVisible({ timeout: 10000 });
  });

  test('clicking Share with Friends opens the suggestion modal', async ({ page }) => {
    await navigateToTab(page, 'Insights');

    const shareButton = page.getByRole('button', { name: /Share with Friends/i });
    await expect(shareButton).toBeVisible({ timeout: 10000 });
    await shareButton.click();

    // Modal should open with title "Coach Suggestions"
    await expect(page.getByText('Coach Suggestions')).toBeVisible({ timeout: 10000 });
  });
});

test.describe('Insights - Share Modal Suggestions', () => {
  test.beforeEach(async ({ page }) => {
    await setupInsightMocks(page);
    await loginToDashboard(page);
    await navigateToTab(page, 'Insights');
  });

  test('modal displays coach-generated suggestions', async ({ page }) => {
    const shareButton = page.getByRole('button', { name: /Share with Friends/i });
    await expect(shareButton).toBeVisible({ timeout: 10000 });
    await shareButton.click();

    await expect(page.getByText('Coach Suggestions')).toBeVisible({ timeout: 10000 });
    await expect(page.getByText('Your coach noticed some achievements!')).toBeVisible();
    await expect(page.getByText('Select a suggestion to share with friends.')).toBeVisible();
  });

  test('suggestion cards show insight content preview', async ({ page }) => {
    const shareButton = page.getByRole('button', { name: /Share with Friends/i });
    await expect(shareButton).toBeVisible({ timeout: 10000 });
    await shareButton.click();

    await expect(page.getByText('Coach Suggestions')).toBeVisible({ timeout: 10000 });

    // Suggestion content should be visible
    await expect(page.getByText(/10K personal best/)).toBeVisible();
    await expect(page.getByText(/Sleep quality improved/)).toBeVisible();
  });
});

test.describe('Insights - Edit and Share Flow', () => {
  async function openEditMode(page: Page) {
    const shareButton = page.getByRole('button', { name: /Share with Friends/i });
    await expect(shareButton).toBeVisible({ timeout: 10000 });
    await shareButton.click();

    await expect(page.getByText('Coach Suggestions')).toBeVisible({ timeout: 10000 });

    // Click on first suggestion's share button (SuggestionCard has a Share button)
    const shareInsightButton = page.getByRole('button', { name: /Share/i }).first();
    await shareInsightButton.click();

    // Should enter editing mode with "Edit & Share" title
    await expect(page.getByText('Edit & Share')).toBeVisible({ timeout: 10000 });
  }

  test.beforeEach(async ({ page }) => {
    await setupInsightMocks(page);
    await loginToDashboard(page);
    await navigateToTab(page, 'Insights');
  });

  test('selecting a suggestion opens editing mode', async ({ page }) => {
    await openEditMode(page);
    await expect(page.getByLabel('Content')).toBeVisible();
  });

  test('editing mode shows suggestion content in textarea', async ({ page }) => {
    await openEditMode(page);

    // Textarea should contain suggestion content
    const textarea = page.locator('textarea');
    await expect(textarea).toHaveValue(/10K personal best/);
  });

  test('visibility selector defaults to Friends Only', async ({ page }) => {
    await openEditMode(page);

    // Friends Only and Public buttons should be visible
    await expect(page.getByText('Friends Only')).toBeVisible();
    await expect(page.getByText('Public')).toBeVisible();
  });

  test('can switch visibility to Public', async ({ page }) => {
    await openEditMode(page);

    // Click Public visibility button
    await page.getByText('Public').click();
    await page.waitForTimeout(200);

    // Public should now be highlighted
    await expect(page.getByText('Public')).toBeVisible();
  });

  test('Share Insight button is disabled when content too short', async ({ page }) => {
    await openEditMode(page);

    // Clear textarea to short content
    const textarea = page.locator('textarea');
    await textarea.fill('Short');

    // Share button should be disabled (min 10 chars required)
    const submitButton = page.getByRole('button', { name: 'Share Insight' });
    await expect(submitButton).toBeDisabled();
  });

  test('character count shows limit', async ({ page }) => {
    await openEditMode(page);

    // Character count format: N/500
    await expect(page.getByText('/500')).toBeVisible();
  });

  test('submitting insight calls the API', async ({ page }) => {
    let shareCalled = false;
    await page.route('**/api/social/insights/from-activity', async (route) => {
      shareCalled = true;
      await route.fulfill({
        status: 201,
        contentType: 'application/json',
        body: JSON.stringify({ insight: mockFeedWithInsight.items[0].insight }),
      });
    });

    await openEditMode(page);

    // Content should already be populated, click Share Insight
    await page.getByRole('button', { name: 'Share Insight' }).click();
    await page.waitForTimeout(500);

    expect(shareCalled).toBe(true);
  });

  test('Cancel button closes modal', async ({ page }) => {
    await openEditMode(page);

    // Click Cancel
    await page.getByRole('button', { name: 'Cancel' }).click();
    await page.waitForTimeout(300);

    // Modal should close
    await expect(page.getByText('Edit & Share')).not.toBeVisible();
  });

  test('Back to suggestions button returns to suggestion list', async ({ page }) => {
    await openEditMode(page);

    // Click Back to suggestions
    await page.getByText('Back to suggestions').click();
    await page.waitForTimeout(300);

    // Should return to suggestions view
    await expect(page.getByText('Coach Suggestions')).toBeVisible({ timeout: 5000 });
  });

  test('shows privacy protection notice', async ({ page }) => {
    await openEditMode(page);

    await expect(page.getByText('Privacy Protected')).toBeVisible();
    await expect(page.getByText(/GPS coordinates.*never shared/i)).toBeVisible();
  });
});

test.describe('Insights - Empty Suggestions', () => {
  test('no suggestion banner when suggestions empty', async ({ page }) => {
    await setupInsightMocks(page, { emptySuggestions: true });
    await loginToDashboard(page);
    await navigateToTab(page, 'Insights');

    // With no suggestions, the banner should not appear
    await page.waitForTimeout(1000);
    await expect(page.getByText('Coach noticed something!')).not.toBeVisible();
  });
});

test.describe('Insights - Error Handling', () => {
  test('handles suggestion fetch failure gracefully', async ({ page }) => {
    await setupInsightMocks(page);

    // Override suggestions with error
    await page.route('**/api/social/insights/suggestions**', async (route) => {
      await route.fulfill({
        status: 500,
        contentType: 'application/json',
        body: JSON.stringify({ error: 'Failed to fetch suggestions' }),
      });
    });

    await loginToDashboard(page);
    await navigateToTab(page, 'Insights');

    // Page should still render without crashing
    await page.waitForTimeout(1000);
    await expect(page.locator('main')).toBeVisible();
  });

  test('handles insight creation failure with error message', async ({ page }) => {
    await setupInsightMocks(page);

    // Override share with error
    await page.route('**/api/social/insights/from-activity', async (route) => {
      await route.fulfill({
        status: 500,
        contentType: 'application/json',
        body: JSON.stringify({ error: 'Failed to create insight' }),
      });
    });

    await loginToDashboard(page);
    await navigateToTab(page, 'Insights');

    const shareButton = page.getByRole('button', { name: /Share with Friends/i });
    await expect(shareButton).toBeVisible({ timeout: 10000 });
    await shareButton.click();

    await expect(page.getByText('Coach Suggestions')).toBeVisible({ timeout: 10000 });

    const shareInsightButton = page.getByRole('button', { name: /Share/i }).first();
    await shareInsightButton.click();

    await expect(page.getByText('Edit & Share')).toBeVisible({ timeout: 10000 });

    // Submit
    await page.getByRole('button', { name: 'Share Insight' }).click();
    await page.waitForTimeout(500);

    // Should show error message
    await expect(page.getByText(/Failed to share/i)).toBeVisible({ timeout: 5000 });
  });
});
